use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BASE_URL: &str = "https://api.jup.ag/prediction/v1";
const USD_SCALE: f64 = 1_000_000.0; // micro-dollars, not cents
// api.jup.ag rate limits at ~5 requests/minute (x-ratelimit-remaining). Three
// filters per refresh means a refresh costs 3 requests, so the TTL cannot drop
// below ~36s without being throttled. 45s = 4 req/min. This is the real ceiling
// on on-chain price freshness, not a tuning choice.
const CACHE_TTL: Duration = Duration::from_secs(45);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

// The venue MUST read the same market universe the matcher indexes. Reading
// only `trending` was the bug that made every signal impossible: 0% of live and
// upcoming market ids appear in the trending payload.
//
// All three filters are required. `trending` looks like the least useful (327
// markets, ~1 usable two-team game) but it is where Jupiter carries the World
// Cup fixture that TxLINE actually streams -- dropping it to save a request
// removed the only match that works end to end.
pub const FILTERS: [&str; 3] = ["live", "upcoming", "trending"];

#[allow(async_fn_in_trait)]
pub trait Venue {
    async fn price(&self, market_id: &str, outcome_id: &str) -> Option<f64>;
}

#[derive(Debug, Deserialize, Clone)]
pub struct EventsResponse {
    pub data: Vec<EventDto>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EventDto {
    pub metadata: Option<EventMetaDto>,
    // Event-level notional in micro-dollars. The market-level `pricing.volume`
    // field is NOT micro-dollars (it reads ~0.9 against an $858k event), so its
    // unit is unconfirmed and it is deliberately not surfaced as money.
    // Jupiter returns this as a JSON number on some events and a quoted string
    // on others. Observed live: 858355000000 and "616716000000".
    #[serde(rename = "volumeUsd", default, deserialize_with = "num_or_string")]
    pub volume_usd: Option<i64>,
    pub markets: Vec<MarketDto>,
}

fn num_or_string<'de, D>(d: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumOrStr {
        N(i64),
        S(String),
        Null,
    }
    Ok(match NumOrStr::deserialize(d)? {
        NumOrStr::N(n) => Some(n),
        NumOrStr::S(s) => s.parse().ok(),
        NumOrStr::Null => None,
    })
}

#[derive(Debug, Deserialize, Clone)]
pub struct EventMetaDto {
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MarketDto {
    #[serde(rename = "marketId")]
    pub market_id: String,
    pub status: Option<String>,
    pub pricing: Option<PricingDto>,
    pub team: Option<TeamDto>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TeamDto {
    pub name: String,
    #[serde(rename = "imageUrl")]
    pub image_url: Option<String>,
    pub color: Option<String>,
    pub abbreviation: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PricingDto {
    #[serde(rename = "buyYesPriceUsd")]
    pub buy_yes_price_usd: Option<i64>,
    #[serde(rename = "buyNoPriceUsd")]
    pub buy_no_price_usd: Option<i64>,
}

// A priced read plus the display metadata the dashboard needs, so the frontend
// never has to invent fields the backend does not send.
#[derive(Debug, Clone)]
pub struct Quote {
    pub price: f64,
    pub volume: Option<f64>,
    pub event_title: Option<String>,
    pub image_url: Option<String>,
    pub color: Option<String>,
    pub abbreviation: Option<String>,
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub struct JupiterPredictVenue {
    client: reqwest::Client,
    cache: Mutex<Option<(Instant, EventsResponse)>>,
}

impl Default for JupiterPredictVenue {
    fn default() -> Self {
        Self::new()
    }
}

impl JupiterPredictVenue {
    pub fn new() -> Self {
        Self { client: client(), cache: Mutex::new(None) }
    }

    // Union of every filter, deduped by marketId. One cached snapshot backs both
    // pricing and the dashboard.
    pub async fn events(&self) -> Option<EventsResponse> {
        {
            let guard = self.cache.lock().ok()?;
            if let Some((fetched_at, resp)) = guard.as_ref() {
                if fetched_at.elapsed() < CACHE_TTL {
                    return Some(resp.clone());
                }
            }
        }

        let mut merged: Vec<EventDto> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for filter in FILTERS {
            let url =
                format!("{BASE_URL}/events?category=sports&filter={filter}&includeMarkets=true");
            let r = match self.client.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("jupiter {filter} request failed: {e}");
                    continue;
                }
            };
            // Check status BEFORE parsing. A 429 body is not JSON, and feeding
            // it to .json() produced a misleading "error decoding response
            // body" that hid the real cause for two runs.
            let status = r.status();
            let remaining = r
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("?")
                .to_string();
            if !status.is_success() {
                eprintln!("jupiter {filter} HTTP {status} (ratelimit remaining {remaining})");
                continue;
            }
            let body = match r.text().await {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("jupiter {filter} read failed: {e}");
                    continue;
                }
            };
            let resp: EventsResponse = match serde_json::from_str(&body) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!(
                        "jupiter {filter} parse error: {e} :: {}",
                        crate::feed::head(&body, 160)
                    );
                    continue;
                }
            };
            for mut e in resp.data {
                e.markets.retain(|m| seen.insert(m.market_id.clone()));
                if !e.markets.is_empty() {
                    merged.push(e);
                }
            }
        }

        // A throttled or failed refresh must not wipe the catalog to zero.
        // Serve the last good snapshot instead.
        if merged.is_empty() {
            eprintln!("jupiter refresh returned nothing; serving last good snapshot");
            let guard = self.cache.lock().ok()?;
            return guard.as_ref().map(|(_, r)| r.clone());
        }
        let out = EventsResponse { data: merged };
        if let Ok(mut g) = self.cache.lock() {
            *g = Some((Instant::now(), out.clone()));
        }
        Some(out)
    }

    pub async fn quote(&self, market_id: &str, outcome_id: &str) -> Option<Quote> {
        let resp = self.events().await?;

        let (event_title, volume, market) = resp.data.iter().find_map(|e| {
            e.markets.iter().find(|m| m.market_id == market_id).map(|m| {
                (
                    e.metadata.as_ref().and_then(|md| md.title.clone()),
                    e.volume_usd.map(|v| v as f64 / USD_SCALE),
                    m.clone(),
                )
            })
        })?;

        // A resolved market reports a zeroed book; reading it would fabricate a
        // divergence. Open markets only.
        if market.status.as_deref() != Some("open") {
            return None;
        }

        let pricing = market.pricing?;
        let micro = match outcome_id {
            "yes" => pricing.buy_yes_price_usd,
            "no" => pricing.buy_no_price_usd,
            _ => None,
        }?;
        if micro <= 0 {
            return None; // a zeroed side is absent liquidity, not a 0% price
        }
        let team = market.team;
        Some(Quote {
            price: micro as f64 / USD_SCALE,
            volume,
            event_title,
            image_url: team.as_ref().and_then(|t| t.image_url.clone()),
            color: team.as_ref().and_then(|t| t.color.clone()),
            abbreviation: team.as_ref().and_then(|t| t.abbreviation.clone()),
        })
    }
}

impl Venue for JupiterPredictVenue {
    async fn price(&self, market_id: &str, outcome_id: &str) -> Option<f64> {
        self.quote(market_id, outcome_id).await.map(|q| q.price)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every live-network test shares ONE snapshot. Fanning out three filters
    // per test across a parallel test binary trips Jupiter's rate limit, and a
    // rate-limited CI run should skip, not report a false failure.
    static SNAPSHOT: tokio::sync::OnceCell<Option<EventsResponse>> =
        tokio::sync::OnceCell::const_new();

    async fn snapshot() -> Option<EventsResponse> {
        SNAPSHOT
            .get_or_init(|| async { JupiterPredictVenue::new().events().await })
            .await
            .clone()
    }

    macro_rules! snap {
        () => {
            match snapshot().await {
                Some(r) => r,
                None => {
                    eprintln!("skipping: jupiter unreachable or rate limited");
                    return;
                }
            }
        };
    }

    #[tokio::test]
    async fn live_read_returns_a_price() {
        let resp = snap!();
        let venue = JupiterPredictVenue::new();
        let market = resp
            .data
            .into_iter()
            .flat_map(|e| e.markets)
            .find(|m| {
                m.status.as_deref() == Some("open")
                    && m.pricing.as_ref().and_then(|p| p.buy_yes_price_usd).unwrap_or(0) > 0
            })
            .expect("no live open market with pricing found");

        let micro = market.pricing.as_ref().and_then(|p| p.buy_yes_price_usd).unwrap();
        let p = micro as f64 / USD_SCALE;
        assert!((0.0..=1.0).contains(&p), "price {p} out of [0,1] for {}", market.market_id);
        let _ = &venue;
    }

    #[tokio::test]
    async fn closed_market_returns_none_not_zero() {
        let resp = snap!();
        let closed = resp
            .data
            .into_iter()
            .flat_map(|e| e.markets)
            .find(|m| m.status.as_deref() == Some("closed"));

        if let Some(market) = closed {
            assert_ne!(market.status.as_deref(), Some("open"));
        }
    }

    #[tokio::test]
    async fn repeated_calls_within_ttl_use_the_cache() {
        let venue = JupiterPredictVenue::new();
        let Some(first) = venue.events().await else {
            eprintln!("skipping: jupiter unreachable");
            return;
        };
        let t = std::time::Instant::now();
        let second = venue.events().await.expect("cached read must not fail");
        assert!(t.elapsed() < std::time::Duration::from_millis(50), "second read hit the network");
        assert_eq!(first.data.len(), second.data.len());
    }

    #[tokio::test]
    async fn venue_universe_covers_every_indexed_game() {
        let resp = snap!();
        let ids: std::collections::HashSet<String> =
            resp.data.iter().flat_map(|e| &e.markets).map(|m| m.market_id.clone()).collect();

        let matcher = crate::mapping::Matcher::from_events(&resp);
        let missing = matcher.market_ids().into_iter().filter(|id| !ids.contains(id)).count();
        assert_eq!(missing, 0, "{missing} indexed markets are unpriceable by the venue");
    }

    #[tokio::test]
    async fn events_dedupes_market_ids_across_filters() {
        let resp = snap!();
        let all: Vec<String> =
            resp.data.iter().flat_map(|e| &e.markets).map(|m| m.market_id.clone()).collect();
        let uniq: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(all.len(), uniq.len(), "duplicate marketIds survived the merge");
    }
}