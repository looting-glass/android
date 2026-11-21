//! Game / realm selection and search settings (platform-independent subset of the desktop config).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Game {
    #[default]
    Poe1,
    Poe2,
}

/// Trade realm. PoE 1 keeps separate economies per platform; PoE 2 has one crossplay realm
/// (`/api/trade2/data/leagues` only lists realm `poe2`), so `realm` is ignored for PoE 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Realm {
    Pc,
    #[default]
    Xbox,
    Sony,
}

impl Game {
    pub fn short(self) -> &'static str {
        match self {
            Game::Poe1 => "PoE 1",
            Game::Poe2 => "PoE 2",
        }
    }
    /// Base URL of the trade API, e.g. `https://www.pathofexile.com/api/trade2`.
    pub fn api(self) -> &'static str {
        match self {
            Game::Poe1 => "https://www.pathofexile.com/api/trade",
            Game::Poe2 => "https://www.pathofexile.com/api/trade2",
        }
    }
    /// Realm id as the trade API names it (`pc`, `xbox`, `sony`, `poe2`).
    pub fn realm_id(self, realm: Realm) -> &'static str {
        match (self, realm) {
            (Game::Poe2, _) => "poe2",
            (Game::Poe1, Realm::Pc) => "pc",
            (Game::Poe1, Realm::Xbox) => "xbox",
            (Game::Poe1, Realm::Sony) => "sony",
        }
    }
    /// Path segment between the endpoint and the league (`xbox/`, `sony/`, `poe2/`; none for PC).
    pub fn realm_path(self, realm: Realm) -> String {
        match self.realm_id(realm) {
            "pc" => String::new(),
            r => format!("{r}/"),
        }
    }
    pub fn web_search(self, realm: Realm, league: &str, id: &str) -> String {
        let seg = match self {
            Game::Poe1 => "trade",
            Game::Poe2 => "trade2",
        };
        format!(
            "https://www.pathofexile.com/{seg}/search/{}{}/{id}",
            self.realm_path(realm),
            urlencode(league)
        )
    }
}

pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Everything a price check needs to know besides the item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub game: Game,
    pub realm: Realm,
    pub league: String,
    /// Only show sellers that are online.
    pub online: bool,
    /// Stat minimums are set to rolled value minus this percentage.
    pub stat_tolerance: u32,
    /// Number of listings to fetch (max 20).
    pub max_results: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            game: Game::Poe1,
            realm: Realm::Xbox,
            league: "Standard".into(),
            online: true,
            stat_tolerance: 10,
            max_results: 10,
        }
    }
}

/// GGG asks API clients to identify themselves.
pub fn user_agent() -> String {
    // LG_VERSION is the app version (version.txt), set by scripts/build-rust.sh.
    format!("LootingGlass-Android/{} (proof of concept)", option_env!("LG_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realm_paths() {
        assert_eq!(Game::Poe1.realm_path(Realm::Pc), "");
        assert_eq!(Game::Poe1.realm_path(Realm::Xbox), "xbox/");
        assert_eq!(Game::Poe2.realm_path(Realm::Sony), "poe2/");
        assert_eq!(
            Game::Poe1.web_search(Realm::Sony, "Hardcore Covens", "abc"),
            "https://www.pathofexile.com/trade/search/sony/Hardcore%20Covens/abc"
        );
    }
}
