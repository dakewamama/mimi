// tests/live_capture.rs
//! Regression test built from a real capture of both venues, taken 2026-07-19
//! against fixture 18257739 (Spain vs Argentina, World Cup, 19:00 UTC).
//!
//! TxLINE /odds/stream, SuperOddsType=1X2_PARTICIPANT_RESULT, MarketPeriod=null:
//!   PriceNames ["part1","draw","part2"], Prices [2375, 3170, 3795]
//!
//! Jupiter Predict (filter=trending):
//!   Spain     POLY-2941974  buyYesPriceUsd 425000
//!   Argentina POLY-2941976  buyYesPriceUsd 270000
//!
//! This drives the same pricing and detection code the agent runs, so the
//! comparison stays verified even when the feed is emitting only first-half
//! books.

use mimi::divergence::{detect, Side};
use mimi::pricing::{devig_proportional, DecimalOdds, Market, Outcome};

const SCALE: f64 = 1000.0;
const THRESHOLD: f64 = 0.03;

fn captured_book() -> Market {
    let raw = [("part1", 2375), ("draw", 3170), ("part2", 3795)];
    Market::new(
        raw.iter()
            .map(|(n, p)| Outcome::new(*n, DecimalOdds::new(*p as f64 / SCALE).unwrap()))
            .collect(),
    )
    .unwrap()
}

#[test]
fn captured_txline_book_is_already_demargined() {
    // Bookmaker is "TXLineStablePriceDemargined": the feed ships a zero-vig
    // book, so de-vigging is a no-op here rather than the edge source the
    // README implies.
    let book = devig_proportional(&captured_book());
    assert!(
        (book.overround - 1.0).abs() < 5e-5,
        "expected a pre-demargined book, got overround {}",
        book.overround
    );
}

#[test]
fn captured_venues_agree_and_no_signal_fires() {
    let book = devig_proportional(&captured_book());
    let spain = book.price_of("part1").unwrap();
    let argentina = book.price_of("part2").unwrap();

    // Jupiter buyYesPriceUsd / 1e6
    let jup_spain = 425_000.0 / 1_000_000.0;
    let jup_argentina = 270_000.0 / 1_000_000.0;

    assert!((spain - 0.4211).abs() < 5e-4, "spain fair {spain}");
    assert!((argentina - 0.2635).abs() < 5e-4, "argentina fair {argentina}");

    assert!(detect("POLY-2941974", "yes", spain, jup_spain, THRESHOLD).is_none());
    assert!(detect("POLY-2941976", "yes", argentina, jup_argentina, THRESHOLD).is_none());

    // Both gaps well inside a point: the venues genuinely agree.
    assert!((spain - jup_spain).abs() < 0.01);
    assert!((argentina - jup_argentina).abs() < 0.01);
}

#[test]
fn the_two_venues_price_the_same_quantity() {
    // The structural risk in this whole design is comparing a 3-way 1X2 fair
    // price against a 2-way on-chain moneyline. If Jupiter's team markets
    // resolve No on a draw, then 1 - yes(Spain) - yes(Argentina) should recover
    // TxLINE's draw price. It does, to about a point -- which is the evidence
    // that the comparison is apples to apples.
    let book = devig_proportional(&captured_book());
    let draw_txline = book.price_of("draw").unwrap();
    let draw_implied_onchain = 1.0 - 0.425 - 0.270;
    assert!(
        (draw_txline - draw_implied_onchain).abs() < 0.02,
        "txline draw {draw_txline} vs on-chain implied {draw_implied_onchain}"
    );
}

#[test]
fn a_first_half_book_would_have_fabricated_a_signal() {
    // The same fixture's MarketPeriod="half=1" book, captured in the same
    // session: Prices [3327, 2045, 4748]. Comparing it against the full-match
    // on-chain price manufactures a 12-point edge out of nothing. This is why
    // main.rs rejects any period-scoped record.
    let half = Market::new(
        [("part1", 3327), ("draw", 2045), ("part2", 4748)]
            .iter()
            .map(|(n, p)| Outcome::new(*n, DecimalOdds::new(*p as f64 / SCALE).unwrap()))
            .collect(),
    )
    .unwrap();
    let spain_half = devig_proportional(&half).price_of("part1").unwrap();

    let bogus = detect("POLY-2941974", "yes", spain_half, 0.425, THRESHOLD)
        .expect("the contaminated comparison does fire, which is the point");
    assert_eq!(bogus.side, Side::Sell);
    assert!(bogus.edge < -0.10, "fabricated edge was {}", bogus.edge);
}