//! edge = fair - venue. Positive edge means the venue underprices the
//! outcome (Buy); negative means it overprices (Sell). `threshold` is a raw
//! price gap; a live gate must also clear fees, slippage, and tip.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Signal {
    pub market_id: String,
    pub outcome_id: String,
    pub fair: f64,
    pub venue: f64,
    pub edge: f64,
    pub side: Side,
}

pub fn detect(
    market_id: &str,
    outcome_id: &str,
    fair: f64,
    venue: f64,
    threshold: f64,
) -> Option<Signal> {
    let edge = fair - venue;
    if edge.abs() < threshold {
        return None;
    }
    let side = if edge > 0.0 { Side::Buy } else { Side::Sell };
    Some(Signal {
        market_id: market_id.to_string(),
        outcome_id: outcome_id.to_string(),
        fair,
        venue,
        edge,
        side,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_signal_below_threshold() {
        assert!(detect("m", "home", 0.50, 0.49, 0.03).is_none());
    }

    #[test]
    fn buy_when_venue_underprices() {
        let s = detect("m", "home", 0.55, 0.48, 0.03).unwrap();
        assert_eq!(s.side, Side::Buy);
        assert!((s.edge - 0.07).abs() < 1e-9);
    }

    #[test]
    fn sell_when_venue_overprices() {
        let s = detect("m", "away", 0.30, 0.40, 0.03).unwrap();
        assert_eq!(s.side, Side::Sell);
        assert!(s.edge < 0.0);
    }

    #[test]
    fn threshold_is_inclusive_boundary() {
        assert!(detect("m", "home", 0.50, 0.47, 0.03).is_some());
    }
}

