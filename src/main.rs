use mimi::api::{self, MatchRow, SignalRecord};
use mimi::divergence::detect;
use mimi::feed::{fixture_teams, MarketSource, TxLineSource};
use mimi::mapping::Matcher;
use mimi::pricing::devig_proportional;
use mimi::venue::JupiterPredictVenue;
use mimi::wallet::LocalSigner;
use std::sync::Arc;
use std::time::Duration;

const THRESHOLD: f64 = 0.03;
// The Jupiter catalog was built once at startup and never refreshed, so games
// that kicked off after launch were invisible for the whole run.
const CATALOG_REFRESH: Duration = Duration::from_secs(90);
const ODDS_TYPE: &str = "1X2_PARTICIPANT_RESULT";

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    match LocalSigner::load() {
        Ok(signer) => println!("signer loaded pubkey={}", signer.pubkey_base58()),
        Err(e) => eprintln!("signer not loaded ({e}) -- detect-only mode"),
    }

    let store = api::new_store();

    let det_store = store.clone();
    tokio::spawn(async move {
        // A panic here used to kill detection while /health stayed green.
        if let Err(e) = tokio::spawn(run_detection(det_store)).await {
            eprintln!("FATAL: detection task died: {e}");
        }
    });

    let addr = std::env::var("MIMI_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    println!("signal feed serving on {addr}");
    if let Err(e) = api::serve(store, &addr).await {
        eprintln!("feed server error: {e}");
    }
}

async fn run_detection(store: api::SignalStore) {
    // .env.example documents TXLINE_JWT as a pin-a-specific-token override, but
    // nothing read it. Honour it, and fall back to a fresh guest token.
    let jwt = match std::env::var("TXLINE_JWT") {
        Ok(t) if !t.trim().is_empty() => {
            println!("txline jwt pinned from TXLINE_JWT ({} chars)", t.len());
            t
        }
        _ => match TxLineSource::guest_jwt().await {
            Ok(t) => {
                println!("txline guest jwt acquired ({} chars)", t.len());
                t
            }
            Err(e) => {
                eprintln!("guest jwt fetch failed: {e}");
                return;
            }
        },
    };

    let api_token = std::env::var("TXLINE_API_TOKEN").unwrap_or_default();
    if api_token.is_empty() {
        eprintln!("TXLINE_API_TOKEN not set; activate first");
        return;
    }

    let fixtures = fixture_teams(&jwt, &api_token).await;
    println!("txline fixtures resolved: {}", fixtures.len());

    // ONE venue instance owns the only Jupiter cache in the process, and the
    // catalog is derived from that same snapshot. Two instances meant two sets
    // of requests against a 5/min budget, and a matcher that could disagree
    // with the pricer about which markets exist.
    let venue = Arc::new(JupiterPredictVenue::new());
    let matcher = Arc::new(tokio::sync::RwLock::new(
        venue.events().await.as_ref().map(Matcher::from_events).unwrap_or_default(),
    ));
    println!("jupiter games indexed: {}", matcher.read().await.game_count());
    {
        let (m, v) = (matcher.clone(), venue.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(CATALOG_REFRESH).await;
                if let Some(resp) = v.events().await {
                    let fresh = Matcher::from_events(&resp);
                    let n = fresh.game_count();
                    *m.write().await = fresh;
                    println!("catalog refreshed: {n} games");
                }
            }
        });
    }

    let mut source = TxLineSource::new(jwt, api_token);
    let mut seen_types: std::collections::HashSet<String> = Default::default();
    let mut seen_periods: std::collections::HashSet<String> = Default::default();

    while let Some(update) = source.next().await {
        api::set_stream_ok(&store, true);

        // Asian Handicap and Over/Under reuse part1/part2 as price labels but
        // price a spread or a goals line, not who wins. Only 1X2 is comparable
        // against Jupiter's moneyline.
        if update.super_odds_type != ODDS_TYPE {
            if seen_types.insert(update.super_odds_type.clone()) {
                println!(
                    "skipping SuperOddsType={} (comparing only {ODDS_TYPE})",
                    update.super_odds_type
                );
            }
            continue;
        }

        // A period-scoped 1X2 ("half=1") prices who leads at the break, not who
        // wins the match. Comparing it against a full-match moneyline
        // manufactures a ~20pp gap out of nothing. Full match only.
        if let Some(period) = &update.market_period {
            if seen_periods.insert(period.clone()) {
                println!("skipping MarketPeriod={period} (full match only)");
            }
            continue;
        }

        let Some(teams) = fixtures.get(&update.fixture_id) else { continue };
        let book = devig_proportional(&update.market);

        for (outcome_id, fair) in &book.prices {
            let team = match outcome_id.as_str() {
                "part1" => &teams.0,
                "part2" => &teams.1,
                _ => continue, // the draw leg has no moneyline counterpart
            };
            let Some(venue_ref) = matcher.read().await.resolve(team) else { continue };
            let Some(quote) = venue.quote(&venue_ref.market_id, venue_ref.side).await else {
                eprintln!("no open jupiter price for {team} ({})", venue_ref.market_id);
                continue;
            };

            // market_id must be the market that was actually priced. It used to
            // be the team name, which made signals untraceable.
            let sig = detect(&venue_ref.market_id, venue_ref.side, *fair, quote.price, THRESHOLD);
            let gap = fair - quote.price;
            let ts = api::now_millis();

            api::upsert_match(
                &store,
                MatchRow {
                    key: format!("{}:{}", update.fixture_id, venue_ref.market_id),
                    ts_millis: ts,
                    team_name: team.clone(),
                    opponent: venue_ref.opponent.clone(),
                    market_id: venue_ref.market_id.clone(),
                    event_title: venue_ref.event_title.clone(),
                    image_url: quote.image_url.clone(),
                    color: quote.color.clone(),
                    abbreviation: quote.abbreviation.clone(),
                    in_running: update.in_running,
                    volume: quote.volume,
                    txline: *fair,
                    jupiter: quote.price,
                    gap,
                    is_signal: sig.is_some(),
                },
            );

            if let Some(s) = sig {
                let fired = api::record(
                    &store,
                    SignalRecord {
                        ts_millis: ts,
                        team_name: team.clone(),
                        opponent: venue_ref.opponent.clone(),
                        event_title: venue_ref.event_title.clone(),
                        image_url: quote.image_url,
                        color: quote.color,
                        abbreviation: quote.abbreviation,
                        in_running: update.in_running,
                        signal: s.clone(),
                    },
                );
                if fired {
                    println!(
                        "signal team={team} txline={:.4} jupiter={:.4} edge={:+.4} side={:?} market={}",
                        s.fair, s.venue, s.edge, s.side, s.market_id
                    );
                }
            }
        }
    }

    api::set_stream_ok(&store, false);
    eprintln!("detection loop exited");
}