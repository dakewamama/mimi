use mimi::api;
use mimi::divergence::detect;
use mimi::feed::{fixture_teams, MarketSource, TxLineSource};
use mimi::mapping::Matcher;
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

    // Resolve fixture ids to team names, and index live Jupiter games.
    let fixtures = fixture_teams(&jwt, &api_token).await;
    println!("txline fixtures resolved: {}", fixtures.len());
    for (fid, (p1, p2)) in fixtures.iter() {
        println!("  fixture {fid}: {p1} vs {p2}");
    }
    let matcher = Matcher::build().await;
    println!("jupiter games indexed: {}", matcher.game_count());

    let mut source = TxLineSource::new(jwt, api_token);
    let venue = JupiterPredictVenue::new();

    while let Some(update) = source.next().await {
        // Asian Handicap and Over/Under markets reuse part1/part2 (or
        // over/under) as price labels, but they price a spread or a goals
        // line, not who wins the match. Comparing those against Jupiter's
        // moneyline market would compare two different quantities and
        // fabricate a divergence that isn't real. Only 1X2 is comparable.
        if update.super_odds_type != "1X2_PARTICIPANT_RESULT" {
            continue;
        }
        let teams = match fixtures.get(&update.fixture_id) {
            Some(t) => t,
            None => continue, // unknown fixture, nothing to match against
        };
        let book = devig_proportional(&update.market);
        for (outcome_id, fair) in &book.prices {
            // Map a TxLINE win outcome to the participant whose win it prices.
            let team = match outcome_id.as_str() {
                "part1" => &teams.0,
                "part2" => &teams.1,
                _ => continue, // draw and non-1X2 outcomes have no moneyline match
            };
            let venue_ref = match matcher.resolve(team) {
                Some(v) => v,
                None => continue,
            };
            match venue.price(&venue_ref.market_id, venue_ref.side).await {
                Some(vp) => {
                    if let Some(s) = detect(team, outcome_id, *fair, vp, THRESHOLD) {
                        println!(
                            "signal team={} txline={:.4} jupiter={:.4} edge={:+.4} side={:?} market={}",
                            team, s.fair, s.venue, s.edge, s.side, venue_ref.market_id
                        );
                        api::record(&store, s);
                    } else {
                        println!(
                            "match team={} txline={:.4} jupiter={:.4} gap={:+.4} (below threshold)",
                            team, fair, vp, fair - vp
                        );
                    }
                }
                None => eprintln!("no open jupiter price for {} ({})", team, venue_ref.market_id),
            }
        }
    }
}