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

