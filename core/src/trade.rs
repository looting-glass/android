//! Official trade API client (pathofexile.com/api/trade for PoE 1, /api/trade2 for PoE 2).

use crate::config::{Game, Settings, urlencode, user_agent};
use crate::data::GameData;
use crate::http::CLIENT;
use crate::item::{Item, Rarity};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    pub price: String,
    pub seller: String,
    pub age: String,
    pub ilvl: Option<u64>,
    pub corrupted: bool,
}

#[derive(Debug, Clone)]
pub struct PriceResult {
    pub total: u64,
    pub listings: Vec<Listing>,
    pub url: String,
}

/// One toggleable search criterion shown in the overlay.
/// `id` is a trade stat id, or `prop:<key>` / `misc:<key>` for item properties.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filter {
    pub id: String,
    pub label: String,
    pub tag: String,
    pub enabled: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// Everything needed to (re)run a search; round-trips through the overlay as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    pub game: Game,
    pub realm: crate::config::Realm,
    pub league: String,
    pub online: bool,
    pub name: Option<String>,
    pub base: Option<String>,
    pub use_base: bool,
    /// Trade category for the item class (e.g. "armour.boots"); searched when `use_base` is off.
    pub category: Option<String>,
    pub corrupted: bool,
    pub filter_corrupted: bool,
    pub class: String,
    pub item_level: Option<u32>,
    pub filters: Vec<Filter>,
}

async fn send(req: reqwest::RequestBuilder) -> Result<Value> {
    let r = req.header(reqwest::header::USER_AGENT, user_agent()).send().await?;
    let status = r.status();
    if status.as_u16() == 429 {
        let wait = r
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("?")
            .to_string();
        bail!("Rate limited by the trade API — retry in {wait}s");
    }
    let body = r.text().await?;
    let v: Value = serde_json::from_str(&body)
        .with_context(|| format!("Unexpected response ({status}) from trade API"))?;
    if !status.is_success() {
        bail!("{}", v["error"]["message"].as_str().unwrap_or(status.as_str()));
    }
    Ok(v)
}

/// League ids for the game and realm.
pub async fn leagues(game: Game, realm: crate::config::Realm) -> Result<Vec<String>> {
    let v = send(CLIENT.get(format!("{}/data/leagues", game.api()))).await?;
    #[derive(Deserialize)]
    struct L {
        id: String,
        realm: Option<String>,
    }
    let want = game.realm_id(realm);
    let ls: Vec<L> = serde_json::from_value(v["result"].clone())?;
    Ok(ls.into_iter().filter(|l| l.realm.as_deref().is_none_or(|r| r == want)).map(|l| l.id).collect())
}

fn age(indexed: &str) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(indexed) else { return String::new() };
    let mins = (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_minutes().max(0);
    match mins {
        0..60 => format!("{mins}m"),
        60..1440 => format!("{}h", mins / 60),
        _ => format!("{}d", mins / 1440),
    }
}

/// Mods worth enabling by default (APT-style heuristic); everything else starts unchecked.
const IMPORTANT: &[&str] = &[
    "Resistance", "maximum Life", "Movement Speed", "all Attributes", "Spirit", "Level of all",
    "maximum Energy Shield", "Critical", "increased Physical Damage", "Attack Speed", "Cast Speed",
    "Allies in your Presence",
];

const PROP_LABELS: &[(&str, &str)] = &[
    ("ar", "Armour"), ("ev", "Evasion Rating"), ("es", "Energy Shield"), ("ward", "Ward"), ("block", "Block"),
];

/// Trade-site category for an "Item Class:" value (same ids on both trade sites; game-only
/// classes simply never appear in the other game's item text).
fn category(class: &str) -> Option<&'static str> {
    Some(match class {
        "Boots" => "armour.boots",
        "Gloves" => "armour.gloves",
        "Helmets" => "armour.helmet",
        "Body Armours" => "armour.chest",
        "Shields" => "armour.shield",
        "Quivers" => "armour.quiver",
        "Foci" => "armour.focus",
        "Bucklers" => "armour.buckler",
        "Rings" => "accessory.ring",
        "Amulets" => "accessory.amulet",
        "Belts" => "accessory.belt",
        "Claws" => "weapon.claw",
        "Daggers" => "weapon.dagger",
        "Rune Daggers" => "weapon.runedagger",
        "One Hand Swords" => "weapon.onesword",
        "Thrusting One Hand Swords" => "weapon.rapier",
        "One Hand Axes" => "weapon.oneaxe",
        "One Hand Maces" => "weapon.onemace",
        "Sceptres" => "weapon.sceptre",
        "Spears" => "weapon.spear",
        "Flails" => "weapon.flail",
        "Two Hand Swords" => "weapon.twosword",
        "Two Hand Axes" => "weapon.twoaxe",
        "Two Hand Maces" => "weapon.twomace",
        "Warstaves" | "Quarterstaves" => "weapon.warstaff",
        "Staves" => "weapon.staff",
        "Bows" => "weapon.bow",
        "Crossbows" => "weapon.crossbow",
        "Wands" => "weapon.wand",
        "Talismans" => "weapon.talisman",
        "Jewels" | "Abyss Jewels" => "jewel",
        _ => return None,
    })
}

/// Build the default search for an item.
pub fn prepare(item: &Item, d: &GameData, cfg: &Settings) -> Spec {
    let base = match item.rarity {
        Rarity::Magic => d.bases.iter().find(|b| item.base.contains(b.as_str())).cloned(),
        _ => Some(item.base.clone()),
    };
    let mut filters = Vec::new();
    let f = |id: String, label: String, tag: &str, enabled: bool, min: Option<f64>| Filter {
        id, label, tag: tag.into(), enabled, min, max: None,
    };

    // The trade site compares armour/evasion/ES and physical DPS at 20% quality.
    let quality = item.quality.unwrap_or(0) as f64;
    let can_quality = !item.corrupted && quality < 20.0;
    let q20 = |v: f64, inc: f64| {
        if can_quality { (v * (120.0 + inc) / (100.0 + quality + inc)).round() } else { v.round() }
    };
    let local_inc = |needle: &str| -> f64 {
        item.mods
            .iter()
            .filter(|m| m.template.starts_with("#% increased ") && !m.template.contains("maximum"))
            .filter(|m| m.template.contains(needle))
            .filter_map(|m| m.values.first())
            .sum()
    };
    let qlabel = |name: &str, raw: f64, adj: f64| {
        if adj != raw.round() { format!("{name}: {raw} (Q20: {adj})") } else { format!("{name}: {raw}") }
    };

    for (key, v) in &item.props {
        let name = PROP_LABELS.iter().find(|(k, _)| k == key).map_or(*key, |(_, l)| *l);
        let adj = match *key {
            "ar" => q20(*v, local_inc("Armour")),
            "ev" => q20(*v, local_inc("Evasion")),
            "es" => q20(*v, local_inc("Energy Shield")),
            _ => *v,
        };
        filters.push(f(format!("prop:{key}"), qlabel(name, *v, adj), "property", false, Some(adj)));
    }

    if let Some(w) = item.weapon.as_ref().filter(|w| w.aps > 0.0) {
        let pdps_raw = (w.phys * w.aps).round();
        let pdps = q20(w.phys, local_inc("Physical Damage")) * w.aps;
        let pdps = pdps.round();
        let edps = (w.ele * w.aps).round();
        let cdps = (w.chaos * w.aps).round();
        let dps = pdps + edps + cdps;
        if w.phys > 0.0 {
            filters.push(f("prop:pdps".into(), qlabel("Physical DPS", pdps_raw, pdps), "weapon", true, Some(pdps)));
        }
        if w.ele > 0.0 {
            filters.push(f("prop:edps".into(), format!("Elemental DPS: {edps}"), "weapon", edps > pdps, Some(edps)));
        }
        filters.push(f("prop:dps".into(), format!("Total DPS: {dps}"), "weapon", false, Some(dps)));
        filters.push(f("prop:aps".into(), format!("Attacks per Second: {:.2}", w.aps), "weapon", false, Some(w.aps)));
        if w.crit > 0.0 {
            filters.push(f("prop:crit".into(), format!("Critical Chance: {:.2}%", w.crit), "weapon", false, Some(w.crit)));
        }
    }
    if let Some(l) = item.gem_level {
        filters.push(f("misc:gem_level".into(), format!("Gem Level: {l}"), "property", true, Some(l as f64)));
    }
    if let (Some(q), true) = (item.quality, item.rarity == Rarity::Gem) {
        filters.push(f("misc:quality".into(), format!("Quality: {q}%"), "property", false, Some(q as f64)));
    }
    if let (Some(l), false) = (item.item_level, matches!(item.rarity, Rarity::Gem | Rarity::Currency)) {
        filters.push(f("misc:ilvl".into(), format!("Item Level: {l}"), "property", false, Some(l as f64)));
    }

    let tol = 1.0 - cfg.stat_tolerance as f64 / 100.0;
    let mut stats = Vec::new();
    for m in &item.mods {
        // PoE 2's trade data drops the leading "+" ("# to maximum Mana"); PoE 1 keeps it, so the
        // fallback is PoE 2-only to avoid matching a different PoE 1 stat.
        let group = m.kind.trade_type().to_string();
        let ids = d.stats.get(&(group.clone(), m.template.clone())).or_else(|| {
            (cfg.game == Game::Poe2)
                .then(|| d.stats.get(&(group, m.template.replace("+#", "#"))))
                .flatten()
        });
        let Some(ids) = ids else {
            log::info!("no trade stat for {} mod: {:?}", m.kind.trade_type(), m.template);
            continue;
        };
        let id = ids.iter().find(|(_, l)| *l == item.has_local_props).unwrap_or(&ids[0]).0.clone();
        let min = (!m.values.is_empty())
            .then(|| m.values.iter().sum::<f64>() / m.values.len() as f64)
            .filter(|avg| *avg > 0.0)
            .map(|avg| (avg * tol).floor());
        let important = IMPORTANT.iter().any(|k| m.template.contains(k));
        stats.push(f(id, m.text.clone(), m.kind.trade_type(), important, min));
    }
    // Few mods: search them all. Uniques: search by name, stats are opt-in for roll checks.
    if stats.len() <= 3 {
        stats.iter_mut().for_each(|s| s.enabled = true);
    }
    if item.rarity == Rarity::Unique {
        stats.iter_mut().for_each(|s| s.enabled = false);
    }
    filters.extend(stats);

    // Items read from a screen have no "Item Class:" line; infer it from the base.
    let class = if item.class.is_empty() { infer_class(d, &item.base) } else { item.class.clone() };
    Spec {
        game: cfg.game,
        realm: cfg.realm,
        league: cfg.league.clone(),
        online: cfg.online,
        name: (item.rarity == Rarity::Unique).then(|| item.name.clone()).flatten(),
        base,
        // Rares are about their mods, not the exact base: search the whole category by
        // default (like Awakened PoE Trade); exact base stays one click away.
        use_base: !(item.rarity == Rarity::Rare && category(&class).is_some()),
        category: category(&class).map(str::to_string),
        corrupted: item.corrupted,
        filter_corrupted: matches!(item.rarity, Rarity::Unique | Rarity::Gem),
        class,
        item_level: item.item_level,
        filters,
    }
}

/// Best-effort "Item Class" for a base type, from the trade item group and name keywords.
pub fn infer_class(d: &GameData, base: &str) -> String {
    let group = d.base_group.get(base).map(String::as_str).unwrap_or_default();
    let has = |ws: &[&str]| ws.iter().any(|w| base.split(' ').any(|b| b == *w));
    let class = match group {
        "accessory" if has(&["Ring"]) => "Rings",
        "accessory" if has(&["Amulet", "Talisman"]) => "Amulets",
        "accessory" if has(&["Belt", "Sash", "Stygian", "Vise"]) => "Belts",
        "jewel" => "Jewels",
        "armour" if has(&["Boots", "Greaves", "Slippers", "Shoes", "Sandals", "Footwraps", "Sabatons", "Treads", "Leggings", "Cuisses"]) => "Boots",
        "armour" if has(&["Gloves", "Gauntlets", "Mitts", "Wraps", "Bracers", "Cuffs", "Vambraces", "Handwraps", "Grips"]) => "Gloves",
        "armour" if has(&["Quiver"]) => "Quivers",
        "armour" if has(&["Focus"]) => "Foci",
        "armour" if has(&["Buckler"]) => "Bucklers",
        "armour" if has(&["Shield", "Kite", "Tower", "Spirit", "Targe", "Bundle"]) && !has(&["Plate", "Robe", "Vest", "Garb"]) => "Shields",
        "armour" if has(&["Helmet", "Helm", "Cap", "Hood", "Mask", "Crown", "Circlet", "Burgonet", "Bascinet", "Sallet", "Cage", "Tricorne", "Pelt", "Coif", "Visage", "Hat", "Casque", "Wreath", "Tiara", "Diadem"]) => "Helmets",
        "armour" => "Body Armours",
        "weapon" if has(&["Claw", "Fist", "Gouger", "Ripper", "Awl", "Paw"]) => "Claws",
        "weapon" if has(&["Bow"]) => "Bows",
        "weapon" if has(&["Crossbow", "Bolter", "Arbalest"]) => "Crossbows",
        "weapon" if has(&["Wand", "Horn"]) => "Wands",
        "weapon" if has(&["Sceptre", "Fetish", "Sekhem", "Rod"]) => "Sceptres",
        "weapon" if has(&["Quarterstaff", "Warstaff"]) => "Warstaves",
        "weapon" if has(&["Staff"]) => "Staves",
        "weapon" if has(&["Spear", "Pike", "Lance"]) => "Spears",
        "weapon" if has(&["Flail"]) => "Flails",
        "weapon" if has(&["Dagger", "Kris", "Stiletto", "Knife", "Poignard", "Trisula", "Skean", "Ambusher", "Boot"]) => "Daggers",
        _ => "",
    };
    class.to_string()
}

fn range(f: &Filter) -> Value {
    let mut v = json!({});
    if let Some(m) = f.min { v["min"] = json!(m) }
    if let Some(m) = f.max { v["max"] = json!(m) }
    v
}

pub fn build_query(spec: &Spec) -> Value {
    let status = if spec.online { "online" } else { "any" };
    let mut q = json!({ "status": { "option": status } });
    if let Some(n) = &spec.name {
        q["name"] = json!(n);
    }
    if let (true, Some(b)) = (spec.use_base, &spec.base) {
        q["type"] = json!(b);
    }
    let category = spec.category.as_ref().filter(|_| !spec.use_base);

    // Filter groups differ between the two trade sites.
    let (def_group, lvl_group) = match spec.game {
        Game::Poe1 => ("armour_filters", "misc_filters"),
        Game::Poe2 => ("equipment_filters", "type_filters"),
    };
    let mut groups: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut put = |group: &str, key: &str, val: Value| {
        groups.entry(group).or_insert_with(|| json!({ "filters": {} }))["filters"][key] = val;
    };
    if let Some(c) = category {
        put("type_filters", "category", json!({ "option": c }));
    }
    if spec.filter_corrupted {
        put("misc_filters", "corrupted", json!({ "option": spec.corrupted.to_string() }));
    }
    let mut stats = Vec::new();
    for f in spec.filters.iter().filter(|f| f.enabled) {
        if let Some(k) = f.id.strip_prefix("prop:") {
            let g = match (k, spec.game) {
                ("pdps" | "edps" | "dps" | "aps" | "crit", Game::Poe1) => "weapon_filters",
                _ => def_group,
            };
            put(g, k, range(f));
        } else if let Some(k) = f.id.strip_prefix("misc:") {
            let g = if k == "gem_level" { "misc_filters" } else { lvl_group };
            put(g, k, range(f));
        } else {
            stats.push(json!({ "id": f.id, "value": range(f) }));
        }
    }
    if !groups.is_empty() {
        q["filters"] = Value::Object(groups);
    }
    if !stats.is_empty() {
        q["stats"] = json!([{ "type": "and", "filters": stats }]);
    }
    json!({ "query": q, "sort": { "price": "asc" } })
}

pub async fn search(spec: &Spec, max_results: u32) -> Result<PriceResult> {
    let game = spec.game;
    let body = build_query(spec);
    log::debug!("query: {body}");

    let search_url = format!("{}/search/{}{}", game.api(), game.realm_path(spec.realm), urlencode(&spec.league));
    let res = send(CLIENT.post(search_url).json(&body)).await?;
    let id = res["id"].as_str().context("search returned no id")?.to_string();
    let total = res["total"].as_u64().unwrap_or(0);
    log::info!("search: {total} matches");
    let hashes: Vec<&str> = res["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(max_results.clamp(1, 20) as usize)
        .collect();

    let mut listings = Vec::new();
    // The fetch endpoint takes at most 10 ids per request.
    for chunk in hashes.chunks(10) {
        let mut url = format!("{}/fetch/{}?query={id}", game.api(), chunk.join(","));
        let realm = game.realm_id(spec.realm);
        if realm != "pc" {
            url.push_str(&format!("&realm={realm}"));
        }
        let fetched = send(CLIENT.get(url)).await?;
        for r in fetched["result"].as_array().into_iter().flatten() {
            let l = &r["listing"];
            let price = match (l["price"]["amount"].as_f64(), l["price"]["currency"].as_str()) {
                (Some(a), Some(c)) => format!("{} {c}", (a * 100.0).round() / 100.0),
                _ => "—".into(),
            };
            listings.push(Listing {
                price,
                seller: l["account"]["name"].as_str().unwrap_or_default().to_string(),
                age: age(l["indexed"].as_str().unwrap_or_default()),
                ilvl: r["item"]["ilvl"].as_u64(),
                corrupted: r["item"]["corrupted"].as_bool().unwrap_or(false),
            });
        }
    }
    Ok(PriceResult { total, listings, url: game.web_search(spec.realm, &spec.league, &id) })
}
