//! Detection loop and signal feed run concurrently. The feed stays up and
//! serves whatever has been detected, independent of live data flow.

use mimi::api;
use mimi::divergence::detect;
use mimi::feed::{MarketSource, TxLineSource};
use mimi::pricing::devig_proportional;
use mimi::venue::{JupiterPredictVenue, Venue};
use mimi::wallet::LocalSigner;

const THRESHOLD: f64 = 0.03;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    match LocalSigner::load() {
        Ok(signer) => println!("signer loaded pubkey={}", signer.pubkey_base58()),
        Err(e) => eprintln!("signer not loaded ({e}) -- detect-only mode"),
    }

    let store = api::new_store();

    let det_store = store.clone();
    tokio::spawn(async move { run_detection(det_store).await });

    let addr = std::env::var("MIMI_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    println!("signal feed serving on {addr}");
    if let Err(e) = api::serve(store, &addr).await {
        eprintln!("feed server error: {e}");
    }
}

async fn run_detection(store: api::SignalStore) {
    let jwt = match TxLineSource::guest_jwt().await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("guest jwt fetch failed: {e}");
            return;
        }
    };
    println!("txline guest jwt acquired ({} chars)", jwt.len());

    let api_token = std::env::var("TXLINE_API_TOKEN").unwrap_or_default();
    if api_token.is_empty() {
        eprintln!("TXLINE_API_TOKEN not set; activate first");
        return;
    }

    let mut source = TxLineSource::new(jwt, api_token);
    let venue = JupiterPredictVenue::new();

    while let Some(update) = source.next().await {
        let book = devig_proportional(&update.market);
        for (outcome_id, fair) in &book.prices {
            match venue.price(&update.market_id, outcome_id).await {
                Some(vp) => {
                    if let Some(s) = detect(&update.market_id, outcome_id, *fair, vp, THRESHOLD) {
                        println!(
                            "signal market={} outcome={} fair={:.4} venue={:.4} edge={:+.4} side={:?}",
                            s.market_id, s.outcome_id, s.fair, s.venue, s.edge, s.side
                        );
                        api::record(&store, s);
                    }
                }
                None => eprintln!("venue: no quote {}/{}", update.market_id, outcome_id),
            }
        }
    }
}

