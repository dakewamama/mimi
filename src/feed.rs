use crate::pricing::{DecimalOdds, Market, Outcome};
use futures_util::StreamExt;
use serde::Deserialize;
use std::env;

const DEVNET_AUTH_ORIGIN: &str = "https://txline-dev.txodds.com";
const DEVNET_API_BASE: &str = "https://txline-dev.txodds.com/api";

// TxODDS integer decimal-odds scale. Confirm against a live event on first run;
// override with TXLINE_PRICE_SCALE if the raw integers say otherwise.
const DEFAULT_PRICE_SCALE: f64 = 1000.0;
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub fn head(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarketUpdate {
    pub market_id: String,
    pub fixture_id: i64,
    pub super_odds_type: String,
    pub in_running: bool,
    pub market_period: Option<String>,
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

pub async fn fixture_teams(
    jwt: &str,
    api_token: &str,
) -> std::collections::HashMap<i64, (String, String)> {
    let url = format!("{DEVNET_API_BASE}/fixtures/snapshot");
    let mut map = std::collections::HashMap::new();
    let resp = crate::venue::client()
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
                eprintln!("fixtures snapshot status {status}: {}", head(&body, 300));
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
                    eprintln!("fixtures snapshot raw (first 500 chars): {}", head(&body, 500));
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
    // "half=1" marks a first-half book. A first-half 1X2 prices who LEADS at
    // half time, not who wins the match, so it must never be compared against a
    // full-match moneyline. Observed live with the draw at 48.9%.
    #[serde(rename = "MarketPeriod")]
    market_period: Option<String>,
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
    dropped: usize,
}

impl TxLineSource {
    pub async fn guest_jwt() -> Result<String, reqwest::Error> {
        let client = crate::venue::client();
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
            dropped: 0,
        }
    }

    async fn connect(&mut self) -> Option<()> {
        let url = format!("{DEVNET_API_BASE}{}", self.odds_path);
        // This is a long-lived SSE stream, not a request/response call. A plain
        // `.timeout()` bounds the WHOLE request including how long the body can
        // stay open, so it was killing a perfectly healthy stream every 15s and
        // forcing a reconnect in a loop forever. `.connect_timeout()` only
        // bounds the TCP/TLS handshake, which is the thing that should time out.
        let resp = reqwest::Client::builder()
            .connect_timeout(HTTP_TIMEOUT)
            .build()
            .ok()?
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
        let legs = rec.price_names.len().min(rec.prices.len());
        let mut outcomes = Vec::new();
        let mut rejected = 0usize;
        for (name, raw) in rec.price_names.iter().zip(rec.prices.iter()) {
            if *raw <= 0 {
                rejected += 1;
                continue;
            }
            let decimal = *raw as f64 / self.price_scale;
            match DecimalOdds::new(decimal) {
                Some(odds) => outcomes.push(Outcome::new(name.clone(), odds)),
                None => rejected += 1,
            }
        }

        // A partial book renormalizes to 1.0 across the surviving legs only,
        // inflating every fair price against a venue that still prices the full
        // book. That fabricates divergence, so refuse the record instead.
        if rejected > 0 {
            self.dropped += 1;
            if self.dropped <= 3 || self.dropped % 100 == 0 {
                eprintln!(
                    "dropped record fixture={} type={} ({rejected}/{legs} legs unusable at scale {}) -- check TXLINE_PRICE_SCALE. total dropped: {}",
                    rec.fixture_id, rec.super_odds_type, self.price_scale, self.dropped
                );
            }
            return None;
        }
        let market = Market::new(outcomes)?;
        let market_id = format!("{}:{}", rec.fixture_id, rec.super_odds_type);
        Some(MarketUpdate {
            market_id,
            fixture_id: rec.fixture_id,
            super_odds_type: rec.super_odds_type.clone(),
            in_running: rec.in_running,
            market_period: rec.market_period.clone(),
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
                // Reconnect with backoff instead of ending the detection loop.
                // Previously any transport hiccup returned None and killed
                // detection for the rest of the process lifetime.
                let mut backoff = std::time::Duration::from_secs(1);
                while self.connect().await.is_none() {
                    eprintln!("odds stream reconnect in {:?}", backoff);
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(std::time::Duration::from_secs(30));
                }
                self.pending.clear();
            }
            let chunk = match self.stream.as_mut()?.next().await {
                Some(Ok(b)) => b,
                Some(Err(e)) => {
                    eprintln!("odds stream error: {e}");
                    self.stream = None;
                    continue;
                }
                None => {
                    eprintln!("odds stream closed by server");
                    self.stream = None;
                    continue;
                }
            };
            self.pending.push_str(&String::from_utf8_lossy(&chunk));
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
    fn first_half_book_is_tagged_not_silently_full_match() {
        let mut src = TxLineSource::new("jwt".into(), "tok".into());
        let json = r#"{"FixtureId":18257739,"SuperOddsType":"1X2_PARTICIPANT_RESULT","InRunning":false,"MarketPeriod":"half=1","PriceNames":["part1","draw","part2"],"Prices":[3327,2045,4748]}"#;
        let u = src.record_to_update(json).expect("should parse");
        assert_eq!(u.market_period.as_deref(), Some("half=1"));
    }

    #[tokio::test]
    async fn live_stream_survives_past_the_old_15s_kill_timer() {
        // Regression test for a real bug: connect() used to build its client
        // with `.timeout(HTTP_TIMEOUT)` (15s), which bounds the WHOLE request
        // including the open body -- so a perfectly healthy SSE stream was
        // getting killed and reconnected every 15s forever. This held one
        // connection open past that mark and confirmed at least one byte still
        // arrived, proving the stream isn't being torn down on a clock.
        let mut src = TxLineSource::new(
            TxLineSource::guest_jwt().await.expect("guest jwt request failed"),
            std::env::var("TXLINE_API_TOKEN").unwrap_or_default(),
        );
        if std::env::var("TXLINE_API_TOKEN").unwrap_or_default().is_empty() {
            eprintln!("skipping: TXLINE_API_TOKEN not set");
            return;
        }
        assert!(src.connect().await.is_some(), "initial connect failed");
        tokio::time::sleep(std::time::Duration::from_secs(17)).await;
        let chunk = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            src.stream.as_mut().unwrap().next(),
        )
        .await;
        assert!(
            matches!(chunk, Ok(Some(Ok(_)))),
            "stream should still be open past the old 15s kill timer, got {chunk:?}"
        );
    }

    #[test]
    fn head_never_panics_on_multibyte() {
        let body = format!("a{}", "\u{20ac}".repeat(400));
        assert!(head(&body, 300).len() <= 300);
        assert_eq!(head("short", 300), "short");
    }

    #[test]
    fn partial_book_is_refused_not_renormalized() {
        let mut src = TxLineSource::new("jwt".into(), "tok".into());
        let json = r#"{"FixtureId":7,"SuperOddsType":"1X2","InRunning":true,"PriceNames":["h","d","a"],"Prices":[2000,0,4000]}"#;
        assert!(src.record_to_update(json).is_none(), "suspended leg must void the book");
    }

    #[test]
    fn wrong_price_scale_drops_loudly() {
        let mut src = TxLineSource::new("jwt".into(), "tok".into());
        src.price_scale = 1_000_000.0; // every decimal now < 1.0
        let json = r#"{"FixtureId":8,"SuperOddsType":"1X2","InRunning":true,"PriceNames":["h","a"],"Prices":[1500,3000]}"#;
        assert!(src.record_to_update(json).is_none());
        assert_eq!(src.dropped, 1, "drop must be counted, not silent");
    }

    #[test]
    fn parses_odds_record_into_market() {
        let mut src = TxLineSource::new("jwt".into(), "tok".into());
        let json = r#"{"FixtureId":123,"SuperOddsType":"1X2","InRunning":true,"PriceNames":["home","draw","away"],"Prices":[2000,3500,4000]}"#;
        let u = src.record_to_update(json).expect("should parse");
        assert_eq!(u.market_id, "123:1X2");
        assert_eq!(u.super_odds_type, "1X2");
        assert!(u.in_running, "in_running must survive into the update");
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