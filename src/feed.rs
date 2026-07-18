use crate::pricing::{DecimalOdds, Market, Outcome};
use futures_util::StreamExt;
use serde::Deserialize;
use std::env;

const DEVNET_AUTH_ORIGIN: &str = "https://txline-dev.txodds.com";
const DEVNET_API_BASE: &str = "https://txline-dev.txodds.com/api";

// TxODDS integer decimal-odds scale. Confirm against a live event on first run;
// override with TXLINE_PRICE_SCALE if the raw integers say otherwise.
const DEFAULT_PRICE_SCALE: f64 = 1000.0;

#[derive(Debug, Clone, PartialEq)]
pub struct MarketUpdate {
    pub market_id: String,
    pub fixture_id: i64,
    pub super_odds_type: String,
    pub market: Market,
}

#[allow(async_fn_in_trait)]
pub trait MarketSource {
    async fn next(&mut self) -> Option<MarketUpdate>;
}

#[derive(Debug, Deserialize)]
struct GuestAuthResponse {
    token: String,
}

#[derive(Debug, Deserialize)]
struct FixtureRecord {
    #[serde(rename = "FixtureId")]
    fixture_id: i64,
    #[serde(rename = "Participant1")]
    participant1: String,
    #[serde(rename = "Participant2")]
    participant2: String,
}

// fixture_id -> (participant1, participant2), fetched once from the fixtures
// snapshot so the matcher can turn a stream fixture id into team names.
// The endpoint returns a bare JSON array, not a {data: [...]} envelope.
pub async fn fixture_teams(
    jwt: &str,
    api_token: &str,
) -> std::collections::HashMap<i64, (String, String)> {
    let url = format!("{DEVNET_API_BASE}/fixtures/snapshot");
    let mut map = std::collections::HashMap::new();
    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(jwt)
        .header("X-Api-Token", api_token)
        .send()
        .await;
    match resp {
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            if !status.is_success() {
                eprintln!("fixtures snapshot status {status}: {}", &body[..body.len().min(300)]);
                return map;
            }
            match serde_json::from_str::<Vec<FixtureRecord>>(&body) {
                Ok(records) => {
                    for f in records {
                        map.insert(f.fixture_id, (f.participant1, f.participant2));
                    }
                }
                Err(e) => {
                    eprintln!("fixtures snapshot parse error: {e}");
                    eprintln!("fixtures snapshot raw (first 500 chars): {}", &body[..body.len().min(500)]);
                }
            }
        }
        Err(e) => eprintln!("fixtures snapshot request failed: {e}"),
    }
    map
}

#[derive(Debug, Deserialize)]
struct OddsRecord {
    #[serde(rename = "FixtureId")]
    fixture_id: i64,
    #[serde(rename = "SuperOddsType")]
    super_odds_type: String,
    #[serde(rename = "InRunning")]
    in_running: bool,
    #[serde(rename = "PriceNames")]
    price_names: Vec<String>,
    #[serde(rename = "Prices")]
    prices: Vec<i32>,
}

pub struct TxLineSource {
    jwt: String,
    api_token: String,
    odds_path: String,
    price_scale: f64,
    buf: std::collections::VecDeque<MarketUpdate>,
    stream: Option<std::pin::Pin<Box<dyn futures_util::Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>>,
    pending: String,
    logged_first: bool,
}

impl TxLineSource {
    pub async fn guest_jwt() -> Result<String, reqwest::Error> {
        let client = reqwest::Client::new();
        let resp: GuestAuthResponse = client
            .post(format!("{DEVNET_AUTH_ORIGIN}/auth/guest/start"))
            .send()
            .await?
            .json()
            .await?;
        Ok(resp.token)
    }

    pub fn new(jwt: String, api_token: String) -> Self {
        let odds_path = env::var("TXLINE_ODDS_PATH").unwrap_or_else(|_| "/odds/stream".to_string());
        let price_scale = env::var("TXLINE_PRICE_SCALE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_PRICE_SCALE);
        Self {
            jwt,
            api_token,
            odds_path,
            price_scale,
            buf: std::collections::VecDeque::new(),
            stream: None,
            pending: String::new(),
            logged_first: false,
        }
    }

    async fn connect(&mut self) -> Option<()> {
        let url = format!("{DEVNET_API_BASE}{}", self.odds_path);
        let resp = reqwest::Client::new()
            .get(&url)
            .bearer_auth(&self.jwt)
            .header("X-Api-Token", &self.api_token)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            eprintln!("odds stream status {}", resp.status());
            return None;
        }
        self.stream = Some(Box::pin(resp.bytes_stream()));
        Some(())
    }

    // Turn one SSE `data:` JSON record into a MarketUpdate. Pairs PriceNames
    // with Prices by index; skips anything that isn't a usable >=2 outcome book.
    fn record_to_update(&mut self, json: &str) -> Option<MarketUpdate> {
        if !self.logged_first {
            println!("txline first odds record: {json}");
            self.logged_first = true;
        }
        let rec: OddsRecord = serde_json::from_str(json).ok()?;
        let mut outcomes = Vec::new();
        for (name, raw) in rec.price_names.iter().zip(rec.prices.iter()) {
            if *raw <= 0 {
                continue;
            }
            let decimal = *raw as f64 / self.price_scale;
            if let Some(odds) = DecimalOdds::new(decimal) {
                outcomes.push(Outcome::new(name.clone(), odds));
            }
        }
        let market = Market::new(outcomes)?;
        let market_id = format!("{}:{}", rec.fixture_id, rec.super_odds_type);
        let _ = rec.in_running;
        Some(MarketUpdate {
            market_id,
            fixture_id: rec.fixture_id,
            super_odds_type: rec.super_odds_type.clone(),
            market,
        })
    }

    fn drain_pending(&mut self) {
        // SSE frames are separated by a blank line; each frame's `data:` lines
        // concatenate into one JSON record.
        while let Some(pos) = self.pending.find("\n\n") {
            let frame: String = self.pending.drain(..pos + 2).collect();
            let mut data = String::new();
            for line in frame.lines() {
                if let Some(rest) = line.strip_prefix("data:") {
                    data.push_str(rest.trim_start());
                }
            }
            if data.is_empty() || data == "{}" {
                continue;
            }
            if let Some(update) = self.record_to_update(&data) {
                self.buf.push_back(update);
            }
        }
    }
}

impl MarketSource for TxLineSource {
    async fn next(&mut self) -> Option<MarketUpdate> {
        loop {
            if let Some(u) = self.buf.pop_front() {
                return Some(u);
            }
            if self.stream.is_none() {
                self.connect().await?;
            }
            let chunk = self.stream.as_mut()?.next().await?;
            let bytes = chunk.ok()?;
            self.pending.push_str(&String::from_utf8_lossy(&bytes));
            self.drain_pending();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn guest_jwt_is_live() {
        let jwt = TxLineSource::guest_jwt().await.expect("guest jwt request failed");
        assert!(!jwt.is_empty());
        assert_eq!(jwt.split('.').count(), 3, "expected a JWT with 3 segments");
    }

    #[test]
    fn parses_odds_record_into_market() {
        let mut src = TxLineSource::new("jwt".into(), "tok".into());
        let json = r#"{"FixtureId":123,"SuperOddsType":"1X2","InRunning":true,"PriceNames":["home","draw","away"],"Prices":[2000,3500,4000]}"#;
        let u = src.record_to_update(json).expect("should parse");
        assert_eq!(u.market_id, "123:1X2");
        assert_eq!(u.super_odds_type, "1X2");
        assert_eq!(u.market.outcomes.len(), 3);
        assert!((u.market.outcomes[0].odds.get() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn sse_framing_yields_a_buffered_update() {
        let mut src = TxLineSource::new("jwt".into(), "tok".into());
        src.pending.push_str("data: {\"FixtureId\":9,\"SuperOddsType\":\"1X2\",\"InRunning\":false,\"PriceNames\":[\"h\",\"a\"],\"Prices\":[1500,3000]}\n\n");
        src.drain_pending();
        assert_eq!(src.buf.len(), 1);
        assert_eq!(src.buf[0].market_id, "9:1X2");
    }
}