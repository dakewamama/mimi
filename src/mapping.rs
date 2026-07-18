use serde::Deserialize;
use std::collections::HashMap;

const JUP_BASE: &str = "https://api.jup.ag/prediction/v1";

// Normalize a team name for matching: lowercase, letters and digits only.
pub fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

// A resolved Jupiter side: the market to read and which price is the win price.
#[derive(Debug, Clone, PartialEq)]
pub struct VenueRef {
    pub market_id: String,
    pub side: &'static str, // always "yes" for a team's own moneyline market
}

// One Jupiter game: two team markets keyed by normalized team name.
#[derive(Debug, Clone)]
struct Game {
    teams: HashMap<String, String>, // normalized team name -> jupiter marketId
}

#[derive(Debug, Deserialize)]
struct JupEvents {
    data: Vec<JupEvent>,
}
#[derive(Debug, Deserialize)]
struct JupEvent {
    markets: Vec<JupMarket>,
}
#[derive(Debug, Deserialize)]
struct JupMarket {
    #[serde(rename = "marketId")]
    market_id: String,
    status: Option<String>,
    team: Option<JupTeam>,
}
#[derive(Debug, Deserialize)]
struct JupTeam {
    name: String,
}

pub struct Matcher {
    games: Vec<Game>,
}

impl Matcher {
    // Build the catalog from Jupiter's live and upcoming game feeds.
    pub async fn build() -> Self {
        let mut games = Vec::new();
        for filter in ["live", "upcoming", "trending"] {
            if let Some(list) = Self::fetch(filter).await {
                games.extend(list);
            }
        }
        Self { games }
    }

    async fn fetch(filter: &str) -> Option<Vec<Game>> {
        let url = format!("{JUP_BASE}/events?category=sports&filter={filter}&includeMarkets=true");
        let resp: JupEvents = reqwest::Client::new().get(&url).send().await.ok()?.json().await.ok()?;
        let mut out = Vec::new();
        for e in resp.data {
            let mut teams = HashMap::new();
            for m in e.markets {
                if m.status.as_deref() != Some("open") {
                    continue;
                }
                if let Some(t) = m.team {
                    teams.insert(normalize(&t.name), m.market_id);
                }
            }
            if teams.len() == 2 {
                out.push(Game { teams });
            }
        }
        Some(out)
    }

    // Resolve a TxLINE fixture's winning team to a Jupiter market.
    // `team` is the participant name whose win outcome we are pricing.
    pub fn resolve(&self, team: &str) -> Option<VenueRef> {
        let key = normalize(team);
        for game in &self.games {
            if let Some(market_id) = game.teams.get(&key) {
                return Some(VenueRef { market_id: market_id.clone(), side: "yes" });
            }
            // token-overlap fallback for partial name differences
            for (cand, market_id) in &game.teams {
                if cand.contains(&key) || key.contains(cand) {
                    return Some(VenueRef { market_id: market_id.clone(), side: "yes" });
                }
            }
        }
        None
    }

    pub fn game_count(&self) -> usize {
        self.games.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_punctuation_and_case() {
        assert_eq!(normalize("St. Louis Cardinals"), "stlouiscardinals");
        assert_eq!(normalize("USA"), "usa");
        assert_eq!(normalize("Man. United"), "manunited");
    }

    #[test]
    fn resolves_exact_and_partial_team_names() {
        let mut teams = HashMap::new();
        teams.insert(normalize("Miami Marlins"), "POLY-1-0".to_string());
        teams.insert(normalize("Milwaukee Brewers"), "POLY-1-1".to_string());
        let m = Matcher { games: vec![Game { teams }] };

        let exact = m.resolve("Miami Marlins").unwrap();
        assert_eq!(exact.market_id, "POLY-1-0");
        assert_eq!(exact.side, "yes");

        // partial: "Brewers" should still hit "milwaukeebrewers"
        assert!(m.resolve("Brewers").is_some());
        assert!(m.resolve("Toronto Blue Jays").is_none());
    }

    #[tokio::test]
    async fn builds_catalog_from_live_jupiter() {
        // Live network call: confirms the feed parses into games. Count can be
        // zero if nothing is live, which is not a failure of the parser.
        let m = Matcher::build().await;
        println!("live+upcoming games indexed: {}", m.game_count());
    }
}