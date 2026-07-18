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

#[cfg(test)]
mod tests {
    use super::*;

    fn odds(v: f64) -> DecimalOdds {
        DecimalOdds::new(v).unwrap()
    }

    #[test]
    fn rejects_odds_below_one() {
        assert!(DecimalOdds::new(0.99).is_none());
        assert!(DecimalOdds::new(f64::NAN).is_none());
        assert!(DecimalOdds::new(f64::INFINITY).is_none());
        assert!(DecimalOdds::new(1.0).is_some());
    }

    #[test]
    fn implied_prob_bounds() {
        assert!((odds(1.0).implied_prob() - 1.0).abs() < EPS);
        assert!((odds(2.0).implied_prob() - 0.5).abs() < EPS);
        let p = odds(4.0).implied_prob();
        assert!(p > 0.0 && p <= 1.0);
    }

    #[test]
    fn market_needs_two_outcomes() {
        let one = vec![Outcome::new("home", odds(2.0))];
        assert!(Market::new(one).is_none());
    }

    #[test]
    fn devig_sums_to_one() {
        let m = Market::new(vec![
            Outcome::new("home", odds(2.00)),
            Outcome::new("draw", odds(3.50)),
            Outcome::new("away", odds(4.00)),
        ])
        .unwrap();

        let book = devig_proportional(&m);
        let sum: f64 = book.prices.iter().map(|(_, p)| p).sum();
        assert!((sum - 1.0).abs() < EPS, "prices summed to {sum}");
        assert!(book.overround > 1.0, "overround was {}", book.overround);
    }

    #[test]
    fn devig_is_monotonic() {
        let m = Market::new(vec![
            Outcome::new("fav", odds(1.50)),
            Outcome::new("dog", odds(3.00)),
        ])
        .unwrap();

        let book = devig_proportional(&m);
        let fav = book.price_of("fav").unwrap();
        let dog = book.price_of("dog").unwrap();
        assert!(fav > dog, "fav {fav} should exceed dog {dog}");
    }

    #[test]
    fn fair_book_has_no_vig() {
        let m = Market::new(vec![
            Outcome::new("yes", odds(2.00)),
            Outcome::new("no", odds(2.00)),
        ])
        .unwrap();

        let book = devig_proportional(&m);
        assert!((book.overround - 1.0).abs() < EPS);
        assert!((book.price_of("yes").unwrap() - 0.5).abs() < EPS);
    }
}

