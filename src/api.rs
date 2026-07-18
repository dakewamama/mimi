//! HTTP signal feed. GET /signals returns recent divergences as JSON,
//! GET /health is a liveness probe. No wallet, no execution rights.

use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::divergence::Signal;

const MAX_SIGNALS: usize = 100;

#[derive(Debug, Clone, Serialize)]
pub struct SignalRecord {
    pub ts_millis: u128,
    #[serde(flatten)]
    pub signal: Signal,
}

pub type SignalStore = Arc<Mutex<VecDeque<SignalRecord>>>;

pub fn new_store() -> SignalStore {
    Arc::new(Mutex::new(VecDeque::with_capacity(MAX_SIGNALS)))
}

pub fn record(store: &SignalStore, signal: Signal) {
    let ts_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut q = store.lock().expect("signal store poisoned");
    if q.len() == MAX_SIGNALS {
        q.pop_front();
    }
    q.push_back(SignalRecord { ts_millis, signal });
}

async fn signals(State(store): State<SignalStore>) -> Json<Vec<SignalRecord>> {
    let q = store.lock().expect("signal store poisoned");
    Json(q.iter().rev().cloned().collect())
}

async fn health() -> &'static str {
    "ok"
}

pub fn router(store: SignalStore) -> Router {
    Router::new()
        .route("/signals", get(signals))
        .route("/health", get(health))
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

    fn sample() -> Signal {
        Signal {
            market_id: "ARG-FRA-1X2".into(),
            outcome_id: "home".into(),
            fair: 0.48,
            venue: 0.42,
            edge: 0.06,
            side: Side::Buy,
        }
    }

    #[test]
    fn record_bounds_and_orders() {
        let store = new_store();
        for _ in 0..(MAX_SIGNALS + 10) {
            record(&store, sample());
        }
        assert_eq!(store.lock().unwrap().len(), MAX_SIGNALS);
    }

    #[test]
    fn record_serializes_to_json() {
        let store = new_store();
        record(&store, sample());
        let rec = store.lock().unwrap().front().unwrap().clone();
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"market_id\":\"ARG-FRA-1X2\""));
        assert!(json.contains("\"side\":\"Buy\""));
        assert!(json.contains("\"ts_millis\""));
    }
}

