//! Kotlin-facing API (UniFFI) over the Looting Glass core.
//!
//! The app does OCR (ML Kit) and colour sampling, then calls `Engine::parse` with the lines,
//! shows the result, and calls `Engine::search` / `Engine::ninja` for prices.

use lootlens_core::config::{Game, Realm, Settings};
use lootlens_core::data::{self, GameData};
use lootlens_core::item::Rarity;
use lootlens_core::{ninja, screen, trade};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

uniffi::setup_scaffolding!();

/// Trade data older than this is refreshed (it changes only with game patches).
const DATA_MAX_AGE: Duration = Duration::from_secs(3 * 24 * 3600);

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum LensError {
    #[error("{0}")]
    Failed(String),
}

impl From<anyhow::Error> for LensError {
    fn from(e: anyhow::Error) -> Self {
        LensError::Failed(format!("{e:#}"))
    }
}

#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum GameId {
    Poe1,
    Poe2,
}

#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum RealmId {
    Pc,
    Xbox,
    Sony,
}

impl From<GameId> for Game {
    fn from(g: GameId) -> Self {
        match g {
            GameId::Poe1 => Game::Poe1,
            GameId::Poe2 => Game::Poe2,
        }
    }
}
impl From<RealmId> for Realm {
    fn from(r: RealmId) -> Self {
        match r {
            RealmId::Pc => Realm::Pc,
            RealmId::Xbox => Realm::Xbox,
            RealmId::Sony => Realm::Sony,
        }
    }
}

/// One OCR line in image pixels. `color` is 0xRRGGBB of the text pixels, if sampled.
#[derive(Debug, Clone, uniffi::Record)]
pub struct OcrLine {
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub color: Option<u32>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ModRow {
    pub text: String,
    pub kind: String,
    pub confidence: f32,
    pub ocr: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct FilterRow {
    pub label: String,
    pub tag: String,
    pub enabled: bool,
    pub min: Option<f64>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Parsed {
    pub title: String,
    pub rarity: String,
    pub rarity_source: String,
    pub base_confidence: f32,
    pub properties: Vec<String>,
    pub mods: Vec<ModRow>,
    /// Lines that were not understood, with their best match score.
    pub unmatched: Vec<String>,
    /// Default search filters, as the desktop overlay would show them.
    pub filters: Vec<FilterRow>,
    /// Clipboard-style text rebuilt from the screen (for debugging / ground-truth comparison).
    pub clipboard: String,
    /// Search spec to hand back to `search` unchanged.
    pub spec_json: String,
    /// For currency, cards and other stackables: the name to look up on poe.ninja.
    pub stackable_name: Option<String>,
    pub stack_size: Option<u32>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ListingRow {
    pub price: String,
    pub seller: String,
    pub age: String,
    pub ilvl: Option<u64>,
    pub corrupted: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PriceRow {
    pub total: u64,
    pub listings: Vec<ListingRow>,
    pub url: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct NinjaRow {
    pub main: String,
    pub detail: String,
}

#[derive(uniffi::Object)]
pub struct Engine {
    cache_dir: PathBuf,
    data: Mutex<HashMap<Game, Arc<GameData>>>,
}

fn prefix(g: Game) -> &'static str {
    match g {
        Game::Poe1 => "trade",
        Game::Poe2 => "trade2",
    }
}

impl Engine {
    fn game_data(&self, g: Game) -> Result<Arc<GameData>, LensError> {
        self.data
            .lock()
            .unwrap()
            .get(&g)
            .cloned()
            .ok_or_else(|| LensError::Failed(format!("{} trade data not loaded", g.short())))
    }

    fn read_cache(&self, g: Game) -> Option<(String, String, SystemTime)> {
        let s = self.cache_dir.join(format!("{}-stats.json", prefix(g)));
        let i = self.cache_dir.join(format!("{}-items.json", prefix(g)));
        let age = std::fs::metadata(&s).and_then(|m| m.modified()).ok()?;
        Some((std::fs::read_to_string(s).ok()?, std::fs::read_to_string(i).ok()?, age))
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Engine {
    #[uniffi::constructor]
    pub fn new(cache_dir: String) -> Arc<Self> {
        Arc::new(Self { cache_dir: PathBuf::from(cache_dir), data: Mutex::new(HashMap::new()) })
    }

    /// Load trade data for a game: from the cache if fresh, else download (falling back to a
    /// stale cache when offline). Returns a short status for the UI.
    pub async fn load(&self, game: GameId) -> Result<String, LensError> {
        let g: Game = game.into();
        if self.data.lock().unwrap().contains_key(&g) {
            return Ok("loaded".into());
        }
        let cached = self.read_cache(g);
        let fresh = cached.as_ref().is_some_and(|(_, _, t)| t.elapsed().is_ok_and(|a| a < DATA_MAX_AGE));
        let (stats, items, status) = match (fresh, cached) {
            (true, Some((s, i, _))) => (s, i, "cached"),
            (_, cached) => match data::fetch_raw(g).await {
                Ok((s, i)) => {
                    let _ = std::fs::create_dir_all(&self.cache_dir);
                    let _ = std::fs::write(self.cache_dir.join(format!("{}-stats.json", prefix(g))), &s);
                    let _ = std::fs::write(self.cache_dir.join(format!("{}-items.json", prefix(g))), &i);
                    (s, i, "downloaded")
                }
                Err(e) => match cached {
                    Some((s, i, _)) => (s, i, "cached (offline)"),
                    None => return Err(e.into()),
                },
            },
        };
        let d = tokio::task::spawn_blocking(move || GameData::from_json(g, &stats, &items))
            .await
            .map_err(|e| LensError::Failed(e.to_string()))??;
        // Build the matcher index now rather than on the first parse.
        let d = Arc::new(d);
        let warm = d.clone();
        let _ = tokio::task::spawn_blocking(move || screen::parse(&[], &warm)).await;
        self.data.lock().unwrap().insert(g, d);
        Ok(status.into())
    }

    /// Parse OCR lines into an item and its default search.
    pub fn parse(&self, game: GameId, realm: RealmId, league: String, online: bool, lines: Vec<OcrLine>) -> Result<Parsed, LensError> {
        let d = self.game_data(game.into())?;
        let lines: Vec<screen::OcrLine> = lines
            .into_iter()
            .map(|l| screen::OcrLine {
                text: l.text,
                left: l.left,
                top: l.top,
                right: l.right,
                bottom: l.bottom,
                color: l.color.map(|c| [(c >> 16) as u8, (c >> 8) as u8, c as u8]),
            })
            .collect();
        let s = screen::parse(&lines, &d).ok_or_else(|| {
            LensError::Failed("No item tooltip found. Fill the frame with the tooltip and keep it sharp.".into())
        })?;
        let settings = Settings { game: game.into(), realm: realm.into(), league, online, ..Settings::default() };
        let spec = trade::prepare(&s.item, &d, &settings);
        let it = &s.item;
        let stackable = matches!(it.rarity, Rarity::Currency | Rarity::DivinationCard | Rarity::Normal | Rarity::Gem | Rarity::Other);
        Ok(Parsed {
            title: it.display_name(),
            rarity: s.report.rarity.clone(),
            rarity_source: s.report.rarity_source.clone(),
            base_confidence: s.report.base_confidence,
            properties: s.report.properties.clone(),
            mods: s
                .report
                .mods
                .iter()
                .map(|m| ModRow { text: m.text.clone(), kind: m.kind.clone(), confidence: m.confidence, ocr: m.ocr.clone() })
                .collect(),
            unmatched: s.report.unmatched.iter().map(|(t, sc)| format!("{t}  ({:.0}%)", sc * 100.0)).collect(),
            filters: spec
                .filters
                .iter()
                .map(|f| FilterRow { label: f.label.clone(), tag: f.tag.clone(), enabled: f.enabled, min: f.min })
                .collect(),
            clipboard: s.report.clipboard.clone(),
            spec_json: serde_json::to_string(&spec).map_err(|e| LensError::Failed(e.to_string()))?,
            stackable_name: stackable.then(|| it.base.clone()),
            stack_size: it.stack_size,
        })
    }

    /// Run the trade search for a spec from `parse`.
    pub async fn search(&self, spec_json: String, max_results: u32) -> Result<PriceRow, LensError> {
        let spec: trade::Spec = serde_json::from_str(&spec_json).map_err(|e| LensError::Failed(e.to_string()))?;
        let r = trade::search(&spec, max_results).await?;
        Ok(PriceRow {
            total: r.total,
            url: r.url,
            listings: r
                .listings
                .into_iter()
                .map(|l| ListingRow { price: l.price, seller: l.seller, age: l.age, ilvl: l.ilvl, corrupted: l.corrupted })
                .collect(),
        })
    }

    /// poe.ninja price for a stackable. poe.ninja tracks the PC economy only.
    pub async fn ninja(&self, game: GameId, league: String, name: String, stack: Option<u32>) -> Option<NinjaRow> {
        let ua = lootlens_core::config::user_agent();
        ninja::lookup(game.into(), &league, &name, stack, &ua).await.map(|(main, detail)| NinjaRow { main, detail })
    }

    pub async fn leagues(&self, game: GameId, realm: RealmId) -> Result<Vec<String>, LensError> {
        Ok(trade::leagues(game.into(), realm.into()).await?)
    }
}
