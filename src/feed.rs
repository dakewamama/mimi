//! TxLINE signal source. `guest_jwt` is a live call needing no wallet;
//! `TxLineSource` streams odds updates into the pricing layer's `Market`.
//! Set TXLINE_ODDS_PATH to the odds endpoint once activated.

use crate::pricing::Market;
use serde::Deserialize;
use std::env;

const DEVNET_AUTH_ORIGIN: &str = "https://txline-dev.txodds.com";
const DEVNET_API_BASE: &str = "https://txline-dev.txodds.com/api";

#[derive(Debug, Clone, PartialEq)]
pub struct MarketUpdate {
    pub market_id: String,
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

pub struct TxLineSource {
    client: reqwest::Client,
    jwt: String,
    api_token: String,
    odds_path: String,
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
        let odds_path = env::var("TXLINE_ODDS_PATH")
            .unwrap_or_else(|_| "/odds/live".to_string());
        Self { client: reqwest::Client::new(), jwt, api_token, odds_path }
    }

    async fn fetch_next(&self) -> Option<MarketUpdate> {
        let url = format!("{DEVNET_API_BASE}{}", self.odds_path);
        let _resp = self
            .client
            .get(&url)
            .bearer_auth(&self.jwt)
            .header("X-Api-Token", &self.api_token)
            .send()
            .await
            .ok()?;
        // Payload parsing lands once TXLINE_ODDS_PATH is confirmed and a
        // sample response is captured.
        None
    }
}

impl MarketSource for TxLineSource {
    async fn next(&mut self) -> Option<MarketUpdate> {
        self.fetch_next().await
    }
}

