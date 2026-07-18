//! Jupiter Predict price read (api.jup.ag/prediction/v1). Read-only, no key.
//! Prices are micro-USD (divide by 1e6 for [0,1]). Only "open" markets are
//! returned; a closed market's zeroed price would fake a divergence.

use serde::Deserialize;

const BASE_URL: &str = "https://api.jup.ag/prediction/v1";
const USD_SCALE: f64 = 1_000_000.0;

#[allow(async_fn_in_trait)]
pub trait Venue {
    async fn price(&self, market_id: &str, outcome_id: &str) -> Option<f64>;
}

#[derive(Debug, Deserialize)]
struct EventsResponse {
    data: Vec<EventDto>,
}

#[derive(Debug, Deserialize)]
struct EventDto {
    markets: Vec<MarketDto>,
}

#[derive(Debug, Deserialize)]
struct MarketDto {
    #[serde(rename = "marketId")]
    market_id: String,
    status: Option<String>,
    pricing: Option<PricingDto>,
}

#[derive(Debug, Deserialize)]
struct PricingDto {
    #[serde(rename = "buyYesPriceUsd")]
    buy_yes_price_usd: Option<i64>,
    #[serde(rename = "buyNoPriceUsd")]
    buy_no_price_usd: Option<i64>,
}

pub struct JupiterPredictVenue {
    client: reqwest::Client,
}

impl JupiterPredictVenue {
    pub fn new() -> Self {
        Self { client: reqwest::Client::new() }
    }

    async fn fetch_price(&self, market_id: &str, outcome_id: &str) -> Option<f64> {
        let url = format!(
            "{BASE_URL}/events?category=sports&filter=trending&includeMarkets=true"
        );
        let resp: EventsResponse = self.client.get(&url).send().await.ok()?.json().await.ok()?;

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
        let url = format!(
            "{BASE_URL}/events?category=sports&filter=trending&includeMarkets=true"
        );
        let resp: EventsResponse = venue
            .client
            .get(&url)
            .send()
            .await
            .expect("events request failed")
            .json()
            .await
            .expect("events response did not parse");

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
        let url = format!(
            "{BASE_URL}/events?category=sports&filter=trending&includeMarkets=true"
        );
        let resp: EventsResponse = venue
            .client
            .get(&url)
            .send()
            .await
            .expect("events request failed")
            .json()
            .await
            .expect("events response did not parse");

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
}

