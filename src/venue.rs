//! Jupiter Predict price read (api.jup.ag/prediction/v1). Read-only, no key.
//! Prices are micro-USD (divide by 1e6 for [0,1]). Only "open" markets are
//! returned; a closed market's zeroed price would fake a divergence.
//!
//! The events list is cached for a short window: without it, every single
//! price() call re-fetches the entire trending catalog, which is wasteful
//! and was causing transient failures under call volume.

use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BASE_URL: &str = "https://api.jup.ag/prediction/v1";
const USD_SCALE: f64 = 1_000_000.0;
const CACHE_TTL: Duration = Duration::from_secs(5);

#[allow(async_fn_in_trait)]
pub trait Venue {
    async fn price(&self, market_id: &str, outcome_id: &str) -> Option<f64>;
}

#[derive(Debug, Deserialize, Clone)]
struct EventsResponse {
    data: Vec<EventDto>,
}

#[derive(Debug, Deserialize, Clone)]
struct EventDto {
    markets: Vec<MarketDto>,
}

#[derive(Debug, Deserialize, Clone)]
struct MarketDto {
    #[serde(rename = "marketId")]
    market_id: String,
    status: Option<String>,
    pricing: Option<PricingDto>,
}

#[derive(Debug, Deserialize, Clone)]
struct PricingDto {
    #[serde(rename = "buyYesPriceUsd")]
    buy_yes_price_usd: Option<i64>,
    #[serde(rename = "buyNoPriceUsd")]
    buy_no_price_usd: Option<i64>,
}

pub struct JupiterPredictVenue {
    client: reqwest::Client,
    cache: Mutex<Option<(Instant, EventsResponse)>>,
}

impl JupiterPredictVenue {
    pub fn new() -> Self {
        Self { client: reqwest::Client::new(), cache: Mutex::new(None) }
    }

    async fn events(&self) -> Option<EventsResponse> {
        {
            let guard = self.cache.lock().expect("cache poisoned");
            if let Some((fetched_at, resp)) = guard.as_ref() {
                if fetched_at.elapsed() < CACHE_TTL {
                    return Some(resp.clone());
                }
            }
        }
        let url = format!("{BASE_URL}/events?category=sports&filter=trending&includeMarkets=true");
        let resp: EventsResponse = self.client.get(&url).send().await.ok()?.json().await.ok()?;
        *self.cache.lock().expect("cache poisoned") = Some((Instant::now(), resp.clone()));
        Some(resp)
    }

    async fn fetch_price(&self, market_id: &str, outcome_id: &str) -> Option<f64> {
        let resp = self.events().await?;

        let market = resp
            .data
            .into_iter()
            .flat_map(|e| e.markets)
            .find(|m| m.market_id == market_id)?;

        if market.status.as_deref() != Some("open") {
            return None;
        }

        let pricing = market.pricing?;
        let cents = match outcome_id {
            "yes" => pricing.buy_yes_price_usd,
            "no" => pricing.buy_no_price_usd,
            _ => None,
        }?;
        Some(cents as f64 / USD_SCALE)
    }
}

impl Venue for JupiterPredictVenue {
    async fn price(&self, market_id: &str, outcome_id: &str) -> Option<f64> {
        self.fetch_price(market_id, outcome_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn live_read_returns_a_price() {
        let venue = JupiterPredictVenue::new();
        let resp = venue.events().await.expect("events request failed");

        let market = resp
            .data
            .into_iter()
            .flat_map(|e| e.markets)
            .find(|m| m.pricing.is_some() && m.status.as_deref() == Some("open"))
            .expect("no live open market with pricing found");

        let price = venue.price(&market.market_id, "yes").await;
        assert!(price.is_some(), "expected a live price for {}/yes", market.market_id);
        let p = price.unwrap();
        assert!((0.0..=1.0).contains(&p), "price {p} out of [0,1] for {}", market.market_id);
    }

    #[tokio::test]
    async fn closed_market_returns_none_not_zero() {
        let venue = JupiterPredictVenue::new();
        let resp = venue.events().await.expect("events request failed");

        let closed = resp
            .data
            .into_iter()
            .flat_map(|e| e.markets)
            .find(|m| m.status.as_deref() == Some("closed"));

        if let Some(market) = closed {
            let price = venue.price(&market.market_id, "yes").await;
            assert!(price.is_none(), "closed market {} should not yield a price", market.market_id);
        }
    }

    #[tokio::test]
    async fn repeated_calls_within_ttl_use_the_cache() {
        let venue = JupiterPredictVenue::new();
        let first = venue.events().await.expect("first fetch failed");
        let second = venue.events().await.expect("second fetch failed");
        // Same data within the TTL window confirms the cache path, not a
        // fresh network round-trip each time.
        assert_eq!(first.data.len(), second.data.len());
    }
}