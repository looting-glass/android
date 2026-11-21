//! Static trade-site data per game: stat texts (`/data/stats`) and item bases (`/data/items`).
//! Downloaded once and cached by the caller; the screen parser matches OCR text against it.

use crate::config::{Game, user_agent};
use crate::http::CLIENT;
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Stat groups a tooltip line can belong to. Others (pseudo, sanctum, crucible…) never show as
/// plain item text lines.
const MOD_GROUPS: &[&str] = &["explicit", "implicit", "crafted", "fractured", "enchant", "rune", "desecrated"];

pub struct GameData {
    pub game: Game,
    /// (stat group, text with `#`) -> [(stat id, is local)]
    pub stats: HashMap<(String, String), Vec<(String, bool)>>,
    /// Known base types, longest first (for resolving magic item names).
    pub bases: Vec<String>,
    /// Base type -> trade item group ("armour", "weapon", "currency", "gem", …).
    pub base_group: HashMap<String, String>,
    /// (unique name, base type)
    pub uniques: Vec<(String, String)>,
    /// Distinct mod stat texts (multi-line texts joined with a space) and the groups they occur in.
    pub mod_texts: Vec<(String, Vec<String>)>,
    pub(crate) matcher: OnceLock<crate::screen::Matcher>,
}

impl GameData {
    pub fn from_json(game: Game, stats_json: &str, items_json: &str) -> Result<Self> {
        let stats: Value = serde_json::from_str(stats_json).context("stats json")?;
        let items: Value = serde_json::from_str(items_json).context("items json")?;
        let mut map: HashMap<(String, String), Vec<(String, bool)>> = HashMap::new();
        let mut texts: HashMap<String, Vec<String>> = HashMap::new();
        for g in stats["result"].as_array().into_iter().flatten() {
            let group = g["id"].as_str().unwrap_or_default();
            for e in g["entries"].as_array().into_iter().flatten() {
                let (Some(id), Some(text)) = (e["id"].as_str(), e["text"].as_str()) else { continue };
                let (text, local) = match text.strip_suffix(" (Local)") {
                    Some(t) => (t, true),
                    None => (text, false),
                };
                map.entry((group.to_string(), text.to_string())).or_default().push((id.to_string(), local));
                if MOD_GROUPS.contains(&group) {
                    let gs = texts.entry(text.to_string()).or_default();
                    if !gs.iter().any(|x| x == group) {
                        gs.push(group.to_string());
                    }
                }
            }
        }
        let mut bases = Vec::new();
        let mut base_group = HashMap::new();
        let mut uniques = Vec::new();
        for g in items["result"].as_array().into_iter().flatten() {
            let group = g["id"].as_str().unwrap_or_default();
            for e in g["entries"].as_array().into_iter().flatten() {
                let Some(ty) = e["type"].as_str() else { continue };
                match e["name"].as_str() {
                    Some(n) => uniques.push((n.to_string(), ty.to_string())),
                    None => {
                        bases.push(ty.to_string());
                        base_group.entry(ty.to_string()).or_insert_with(|| group.to_string());
                    }
                }
            }
        }
        bases.sort_by_key(|b| std::cmp::Reverse(b.len()));
        bases.dedup();
        let mut mod_texts: Vec<(String, Vec<String>)> = texts.into_iter().collect();
        mod_texts.sort();
        log::info!("{} trade data: {} stats, {} bases, {} uniques", game.short(), map.len(), bases.len(), uniques.len());
        Ok(Self { game, stats: map, bases, base_group, uniques, mod_texts, matcher: OnceLock::new() })
    }
}

/// Download raw `/data/stats` and `/data/items` JSON (cache them; they change only with patches).
pub async fn fetch_raw(game: Game) -> Result<(String, String)> {
    let get = |what: &'static str| async move {
        let r = CLIENT
            .get(format!("{}/data/{what}", game.api()))
            .header(reqwest::header::USER_AGENT, user_agent())
            .send()
            .await?
            .error_for_status()?;
        anyhow::Ok(r.text().await?)
    };
    let (s, i) = tokio::try_join!(get("stats"), get("items"))?;
    Ok((s, i))
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use std::sync::LazyLock;

    pub static POE1: LazyLock<GameData> = LazyLock::new(|| {
        GameData::from_json(
            Game::Poe1,
            include_str!("../data/trade-stats.json"),
            include_str!("../data/trade-items.json"),
        )
        .unwrap()
    });
    pub static POE2: LazyLock<GameData> = LazyLock::new(|| {
        GameData::from_json(
            Game::Poe2,
            include_str!("../data/trade2-stats.json"),
            include_str!("../data/trade2-items.json"),
        )
        .unwrap()
    });
}
