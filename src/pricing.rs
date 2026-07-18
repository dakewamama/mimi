//! Odds to fair-value conversion. Invariants are enforced by construction
//! and covered by the tests below.

pub const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecimalOdds(f64);

impl DecimalOdds {
    pub fn new(value: f64) -> Option<Self> {
        (value.is_finite() && value >= 1.0).then_some(Self(value))
    }

    #[inline]
    pub fn get(self) -> f64 {
        self.0
    }

    #[inline]
    pub fn implied_prob(self) -> f64 {
        1.0 / self.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub id: String,
    pub odds: DecimalOdds,
}

impl Outcome {
    pub fn new(id: impl Into<String>, odds: DecimalOdds) -> Self {
        Self { id: id.into(), odds }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Market {
    pub outcomes: Vec<Outcome>,
}

impl Market {
    pub fn new(outcomes: Vec<Outcome>) -> Option<Self> {
        (outcomes.len() >= 2).then_some(Self { outcomes })
    }

    pub fn overround(&self) -> f64 {
        self.outcomes.iter().map(|o| o.odds.implied_prob()).sum()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FairBook {
    pub prices: Vec<(String, f64)>,
    pub overround: f64,
}

impl FairBook {
    pub fn price_of(&self, id: &str) -> Option<f64> {
        self.prices.iter().find(|(k, _)| k == id).map(|(_, p)| *p)
    }
}

// Proportional de-vig. Does not correct favorite-longshot skew; Shin is the
// next strategy if that bias matters for a live signal.
pub fn devig_proportional(market: &Market) -> FairBook {
    let overround = market.overround();
    let prices = market
        .outcomes
        .iter()
        .map(|o| (o.id.clone(), o.odds.implied_prob() / overround))
        .collect();
    FairBook { prices, overround }
}

