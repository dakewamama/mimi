use crate::venue::{EventsResponse, JupiterPredictVenue};

// Normalize a team name for matching: lowercase, letters and digits only.
pub fn normalize(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

// A resolved Jupiter side: the market to read, which price is the win price,
// and the context the dashboard needs to render the row.
#[derive(Debug, Clone, PartialEq)]
pub struct VenueRef {
    pub market_id: String,
    pub side: &'static str, // always "yes" for a team's own moneyline market
    pub opponent: String,
    pub event_title: Option<String>,
}

// One Jupiter game: two team markets keyed by normalized team name.
#[derive(Debug, Clone)]
pub struct Game {
    teams: Vec<(String, String, String)>, // (normalized, display, marketId)
    title: Option<String>,
}

impl Game {
    fn other(&self, idx: usize) -> String {
        self.teams.iter().enumerate().find(|(i, _)| *i != idx).map(|(_, t)| t.1.clone()).unwrap_or_default()
    }
}

#[derive(Default)]
pub struct Matcher {
    games: Vec<Game>,
}

impl Matcher {
    // Built from the venue's cached union snapshot, so the matcher and the
    // pricer can never disagree about which markets exist.
    pub async fn build() -> Self {
        let venue = JupiterPredictVenue::new();
        match venue.events().await {
            Some(resp) => Self::from_events(&resp),
            None => Self::default(),
        }
    }

    pub fn from_events(resp: &EventsResponse) -> Self {
        let mut games = Vec::new();
        for e in &resp.data {
            let mut teams = Vec::new();
            for m in &e.markets {
                if m.status.as_deref() != Some("open") {
                    continue;
                }
                if let Some(t) = &m.team {
                    teams.push((normalize(&t.name), t.name.clone(), m.market_id.clone()));
                }
            }
            if teams.len() == 2 {
                games.push(Game { teams, title: e.metadata.as_ref().and_then(|m| m.title.clone()) });
            }
        }
        Self { games }
    }

    // Exact match wins across the WHOLE catalog before any fuzzy match is tried,
    // and fuzzy matches must be unambiguous. A wrong resolve prices the wrong
    // market and emits a confidently wrong signal, which is worse than None.
    pub fn resolve(&self, team: &str) -> Option<VenueRef> {
        let key = normalize(team);
        if key.len() < 3 {
            return None;
        }

        for game in &self.games {
            if let Some(i) = game.teams.iter().position(|(n, _, _)| *n == key) {
                return Some(self.venue_ref(game, i));
            }
        }

        let mut hit = None;
        for game in &self.games {
            for (i, (cand, _, _)) in game.teams.iter().enumerate() {
                if cand.contains(&key) || key.contains(cand.as_str()) {
                    if hit.is_some() {
                        return None; // ambiguous: refuse rather than guess
                    }
                    hit = Some((game, i));
                }
            }
        }
        hit.map(|(g, i)| self.venue_ref(g, i))
    }

    fn venue_ref(&self, game: &Game, idx: usize) -> VenueRef {
        VenueRef {
            market_id: game.teams[idx].2.clone(),
            side: "yes",
            opponent: game.other(idx),
            event_title: game.title.clone(),
        }
    }

    pub fn game_count(&self) -> usize {
        self.games.len()
    }

    pub fn market_ids(&self) -> Vec<String> {
        self.games.iter().flat_map(|g| g.teams.iter().map(|t| t.2.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(a: (&str, &str), b: (&str, &str)) -> Game {
        Game {
            teams: vec![
                (normalize(a.0), a.0.to_string(), a.1.to_string()),
                (normalize(b.0), b.0.to_string(), b.1.to_string()),
            ],
            title: Some("Test Event".into()),
        }
    }

    #[test]
    fn normalize_strips_punctuation_and_case() {
        assert_eq!(normalize("St. Louis Cardinals"), "stlouiscardinals");
        assert_eq!(normalize("USA"), "usa");
        assert_eq!(normalize("Man. United"), "manunited");
    }

    #[test]
    fn resolves_exact_and_partial_team_names() {
        let m = Matcher {
            games: vec![game(("Miami Marlins", "POLY-1-0"), ("Milwaukee Brewers", "POLY-1-1"))],
        };
        let exact = m.resolve("Miami Marlins").unwrap();
        assert_eq!(exact.market_id, "POLY-1-0");
        assert_eq!(exact.side, "yes");
        assert_eq!(exact.opponent, "Milwaukee Brewers");
        assert!(m.resolve("Brewers").is_some());
        assert!(m.resolve("Toronto Blue Jays").is_none());
    }

    #[test]
    fn exact_match_beats_an_earlier_fuzzy_match() {
        let m = Matcher {
            games: vec![
                game(("Manchester City", "M-1"), ("Norwich City", "M-2")),
                game(("City", "M-3"), ("Rovers", "M-4")),
            ],
        };
        assert_eq!(m.resolve("City").unwrap().market_id, "M-3");
    }

    #[test]
    fn ambiguous_fuzzy_match_refuses_rather_than_guessing() {
        let m = Matcher {
            games: vec![game(("Manchester City", "M-1"), ("Norwich City", "M-2"))],
        };
        assert!(m.resolve("City FC").is_none() || m.resolve("Cit").is_none());
        assert!(m.resolve("ci").is_none(), "2-char keys must not resolve");
    }

    #[tokio::test]
    async fn builds_catalog_from_live_jupiter() {
        let m = Matcher::build().await;
        println!("live games indexed: {}", m.game_count());
    }
}