use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tower_http::cors::{Any, CorsLayer};

use crate::divergence::Signal;

const MAX_SIGNALS: usize = 100;
// A persistent divergence re-fires on every stream tick. Without a cooldown the
// 100-slot ring fills with copies of one signal within seconds.
const DEDUPE_MS: u128 = 30_000;

pub fn now_millis() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalRecord {
    pub ts_millis: u128,
    pub team_name: String,
    pub opponent: String,
    pub event_title: Option<String>,
    pub image_url: Option<String>,
    pub color: Option<String>,
    pub abbreviation: Option<String>,
    pub in_running: bool,
    #[serde(flatten)]
    pub signal: Signal,
}

// One tracked comparison, whether or not it cleared threshold.
#[derive(Debug, Clone, Serialize)]
pub struct MatchRow {
    pub key: String,
    pub ts_millis: u128,
    pub team_name: String,
    pub opponent: String,
    pub market_id: String,
    pub event_title: Option<String>,
    pub image_url: Option<String>,
    pub color: Option<String>,
    pub abbreviation: Option<String>,
    pub in_running: bool,
    pub volume: Option<f64>,
    pub txline: f64,
    pub jupiter: f64,
    pub gap: f64,
    pub is_signal: bool,
}

#[derive(Default)]
pub struct Store {
    pub signals: VecDeque<SignalRecord>,
    pub matches: HashMap<String, MatchRow>,
    pub last_fired: HashMap<String, u128>,
    pub started_ms: u128,
    pub stream_ok: bool,
}

pub type SignalStore = Arc<Mutex<Store>>;

pub fn new_store() -> SignalStore {
    Arc::new(Mutex::new(Store { started_ms: now_millis(), ..Default::default() }))
}

// Returns false if the signal was suppressed as a duplicate within the cooldown.
pub fn record(store: &SignalStore, rec: SignalRecord) -> bool {
    let Ok(mut s) = store.lock() else { return false };
    let key = format!("{}|{}|{:?}", rec.signal.market_id, rec.signal.outcome_id, rec.signal.side);
    if let Some(prev) = s.last_fired.get(&key) {
        if rec.ts_millis.saturating_sub(*prev) < DEDUPE_MS {
            return false;
        }
    }
    s.last_fired.insert(key, rec.ts_millis);
    if s.signals.len() >= MAX_SIGNALS {
        s.signals.pop_front();
    }
    s.signals.push_back(rec);
    true
}

pub fn upsert_match(store: &SignalStore, row: MatchRow) {
    if let Ok(mut s) = store.lock() {
        s.matches.insert(row.key.clone(), row);
    }
}

pub fn set_stream_ok(store: &SignalStore, ok: bool) {
    if let Ok(mut s) = store.lock() {
        s.stream_ok = ok;
    }
}

async fn signals(State(store): State<SignalStore>) -> Json<Vec<SignalRecord>> {
    match store.lock() {
        Ok(s) => Json(s.signals.iter().rev().cloned().collect()),
        Err(_) => Json(Vec::new()),
    }
}

async fn matches(State(store): State<SignalStore>) -> Json<Vec<MatchRow>> {
    match store.lock() {
        Ok(s) => {
            let mut v: Vec<MatchRow> = s.matches.values().cloned().collect();
            v.sort_by(|a, b| {
                b.in_running
                    .cmp(&a.in_running)
                    .then(b.gap.abs().partial_cmp(&a.gap.abs()).unwrap_or(std::cmp::Ordering::Equal))
            });
            Json(v)
        }
        Err(_) => Json(Vec::new()),
    }
}

#[derive(Serialize)]
struct Status {
    ok: bool,
    stream_ok: bool,
    uptime_ms: u128,
    tracked: usize,
    signals: usize,
}

async fn status(State(store): State<SignalStore>) -> Json<Status> {
    match store.lock() {
        Ok(s) => Json(Status {
            ok: true,
            stream_ok: s.stream_ok,
            uptime_ms: now_millis().saturating_sub(s.started_ms),
            tracked: s.matches.len(),
            signals: s.signals.len(),
        }),
        Err(_) => Json(Status { ok: false, stream_ok: false, uptime_ms: 0, tracked: 0, signals: 0 }),
    }
}

async fn health() -> &'static str {
    "ok"
}

pub fn router(store: SignalStore) -> Router {
    Router::new()
        .route("/signals", get(signals))
        .route("/matches", get(matches))
        .route("/status", get(status))
        .route("/health", get(health))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .with_state(store)
}

pub async fn serve(store: SignalStore, addr: &str) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(store)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::divergence::Side;

    fn sample(market: &str, ts: u128) -> SignalRecord {
        SignalRecord {
            ts_millis: ts,
            team_name: "Argentina".into(),
            opponent: "France".into(),
            event_title: Some("Argentina vs France".into()),
            image_url: None,
            color: None,
            abbreviation: Some("ARG".into()),
            in_running: true,
            signal: Signal {
                market_id: market.into(),
                outcome_id: "yes".into(),
                fair: 0.48,
                venue: 0.42,
                edge: 0.06,
                side: Side::Buy,
            },
        }
    }

    #[test]
    fn record_bounds_and_orders() {
        let store = new_store();
        for i in 0..(MAX_SIGNALS + 10) {
            record(&store, sample(&format!("POLY-{i}"), 1_000 + i as u128));
        }
        assert_eq!(store.lock().unwrap().signals.len(), MAX_SIGNALS);
    }

    #[test]
    fn duplicate_within_cooldown_is_suppressed() {
        let store = new_store();
        assert!(record(&store, sample("POLY-1", 1_000)));
        assert!(!record(&store, sample("POLY-1", 1_500)), "repeat must be suppressed");
        assert!(record(&store, sample("POLY-1", 1_000 + DEDUPE_MS + 1)), "cooldown must expire");
        assert_eq!(store.lock().unwrap().signals.len(), 2);
    }

    #[test]
    fn record_serializes_with_the_fields_the_dashboard_reads() {
        let store = new_store();
        record(&store, sample("POLY-1", 1_000));
        let rec = store.lock().unwrap().signals.front().unwrap().clone();
        let json = serde_json::to_string(&rec).unwrap();
        for field in
            ["market_id", "team_name", "opponent", "event_title", "abbreviation", "side", "ts_millis"]
        {
            assert!(json.contains(&format!("\"{field}\"")), "missing {field} in {json}");
        }
        assert!(json.contains("\"market_id\":\"POLY-1\""));
        assert!(json.contains("\"side\":\"Buy\""));
    }

    #[test]
    fn matches_upsert_by_key_not_append() {
        let store = new_store();
        let mut row = MatchRow {
            key: "k".into(),
            ts_millis: 1,
            team_name: "A".into(),
            opponent: "B".into(),
            market_id: "M".into(),
            event_title: None,
            image_url: None,
            color: None,
            abbreviation: None,
            in_running: true,
            volume: None,
            txline: 0.5,
            jupiter: 0.5,
            gap: 0.0,
            is_signal: false,
        };
        upsert_match(&store, row.clone());
        row.gap = 0.1;
        upsert_match(&store, row);
        let s = store.lock().unwrap();
        assert_eq!(s.matches.len(), 1);
        assert!((s.matches["k"].gap - 0.1).abs() < 1e-9);
    }
}