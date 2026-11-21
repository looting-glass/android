//! Fuzzy parser for item tooltips read by OCR from a screenshot or a photo of a screen.
//!
//! The clipboard text has exact lines, `--------` separators and a `Rarity:` line. On screen
//! there's only noisy text with bounding boxes and colours. This module:
//!
//! 1. orders OCR lines and splits them into sections at large vertical gaps (the drawn dividers);
//! 2. reads name / base from the header, fuzzy-matched against the trade site's item list, and
//!    rarity from the header colour (falling back to the shape of the header);
//! 3. normalizes properties (`Evasion Rating: 260`, `Quality: +20%`, …) by fuzzy label matching;
//! 4. matches every other line against the trade site's ~18k stat texts (trigram shortlist, then
//!    edit distance), which corrects OCR mistakes instead of just reading them;
//! 5. rebuilds clipboard-style text and runs the normal clipboard parser on it, so the result is
//!    the same [`Item`] the desktop app works with.

use crate::data::GameData;
use crate::item::{self, Item, Rarity};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::LazyLock;

/// One line of OCR output, in image pixel coordinates.
#[derive(Debug, Clone)]
pub struct OcrLine {
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    /// Average colour of the text pixels inside the box, if the caller sampled it.
    pub color: Option<[u8; 3]>,
}

/// Minimum score for a line to be accepted as a mod.
pub const MOD_THRESHOLD: f32 = 0.78;

#[derive(Debug, Clone, Serialize)]
pub struct ModMatch {
    /// OCR text the match was made from (two lines joined if the mod wrapped).
    pub ocr: String,
    /// Corrected line, as it would appear on the clipboard.
    pub text: String,
    pub kind: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Report {
    pub name: Option<String>,
    pub base: String,
    pub rarity: String,
    /// How rarity was decided: "colour", "unique name", "base match", "header shape".
    pub rarity_source: String,
    pub base_confidence: f32,
    pub mods: Vec<ModMatch>,
    pub properties: Vec<String>,
    /// Lines that were neither header, property, flag nor an accepted mod (flavour text, OCR junk,
    /// mods below the threshold). Each with its best-guess score.
    pub unmatched: Vec<(String, f32)>,
    /// The clipboard-style text handed to the clipboard parser.
    pub clipboard: String,
}

pub struct ScreenItem {
    pub item: Item,
    pub report: Report,
}

// ---------------------------------------------------------------- text normalization

static NUMERICISH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[+\-]?[0-9OoIl|SBZ]*[0-9][0-9OoIl|SBZ.,]*%?").unwrap());
static NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:\.\d+)?").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
/// Small-caps "TO" read as "10": a standalone token, or glued to the number before it.
static TEN_AS_TO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|[\s\d%])(?:10|1O|T0) ([A-Za-z#\d])").unwrap());

/// The line as read, plus a variant with every likely "TO"->"10" misread undone.
fn readings(line: &str) -> Vec<String> {
    let mut v = vec![line.to_string()];
    let fixed = TEN_AS_TO.replace_all(line, "$1 to $2");
    let fixed = SPACES.replace_all(&fixed, " ").into_owned();
    if fixed != line {
        v.push(fixed);
    }
    v
}

/// Fix letter/digit confusions inside tokens that contain at least one digit (`1O%` -> `10%`,
/// `l2` -> `12`), unify dashes and quotes, collapse spaces.
/// OCR sometimes adds accents to plain capitals ("INCREAŞED", "TỚ"); map them back to ASCII.
fn fold_accents(s: &str) -> String {
    const MAP: &[(&str, char)] = &[
        ("àáâãäåāăąạảấầẩẫậắằẳẵặ", 'a'), ("çćĉċč", 'c'), ("ďđ", 'd'), ("èéêëēĕėęěẹẻẽếềểễệ", 'e'),
        ("ĝğġģ", 'g'), ("ĥħ", 'h'), ("ìíîïĩīĭįıỉị", 'i'), ("ĵ", 'j'), ("ķ", 'k'), ("ĺļľŀł", 'l'),
        ("ñńņňŉ", 'n'), ("òóôõöøōŏőơọỏốồổỗộớờởỡợ", 'o'), ("ŕŗř", 'r'), ("śŝşšș", 's'), ("ţťŧț", 't'),
        ("ùúûüũūŭůűųưụủứừửữự", 'u'), ("ŵ", 'w'), ("ýÿŷỳỵỷỹ", 'y'), ("źżž", 'z'),
    ];
    s.chars()
        .map(|c| {
            if c.is_ascii() {
                return c;
            }
            let lower = c.to_lowercase().next().unwrap_or(c);
            match MAP.iter().find(|(set, _)| set.contains(lower)) {
                Some((_, a)) if c.is_uppercase() => a.to_ascii_uppercase(),
                Some((_, a)) => *a,
                None => c,
            }
        })
        .collect()
}

pub fn clean_ocr(s: &str) -> String {
    let s = fold_accents(s);
    let s = s
        .replace(['\u{2013}', '\u{2014}', '\u{2212}'], "-")
        .replace(['\u{2018}', '\u{2019}', '`'], "'")
        .replace(['\u{201C}', '\u{201D}'], "\"");
    let s = NUMERICISH.replace_all(&s, |c: &regex::Captures| {
        c[0].chars()
            .map(|ch| match ch {
                'O' | 'o' => '0',
                'I' | 'l' | '|' => '1',
                'S' => '5',
                'B' => '8',
                'Z' => '2',
                ',' => '.',
                c => c,
            })
            .collect::<String>()
    });
    // OCR often reads "%" as "9%" / "%%" or puts a space before it.
    let s = s.replace(" %", "%").replace("%%", "%");
    SPACES.replace_all(s.trim(), " ").into_owned()
}

/// Matching key: lowercase letters, digits replaced by `#`, `%` kept, sign and punctuation dropped.
fn key(s: &str) -> String {
    let t = NUM.replace_all(s, "#");
    let mut out = String::with_capacity(t.len());
    for ch in t.chars() {
        match ch {
            'a'..='z' | '#' | '%' | ' ' => out.push(ch),
            'A'..='Z' => out.push(ch.to_ascii_lowercase()),
            '\n' => out.push(' '),
            _ => {}
        }
    }
    SPACES.replace_all(out.trim(), " ").into_owned()
}

fn lev(a: &[u8], b: &[u8]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

pub fn similarity(a: &str, b: &str) -> f32 {
    let n = a.len().max(b.len());
    if n == 0 {
        return 1.0;
    }
    1.0 - lev(a.as_bytes(), b.as_bytes()) as f32 / n as f32
}

/// Best match of `needle` anywhere inside `hay` (semi-global edit distance), as a 0..1 score.
fn substring_similarity(needle: &str, hay: &str) -> f32 {
    let (a, b) = (needle.as_bytes(), hay.as_bytes());
    if a.is_empty() {
        return 0.0;
    }
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let best = *prev.iter().min().unwrap();
    1.0 - best as f32 / a.len() as f32
}

// ---------------------------------------------------------------- stat matcher

/// Trigram index over the trade site's stat texts.
pub struct Matcher {
    /// (key, original text, groups)
    entries: Vec<(String, String, Vec<String>)>,
    trigrams: HashMap<[u8; 3], Vec<u32>>,
    tri_count: Vec<u16>,
}

fn trigrams(k: &str) -> Vec<[u8; 3]> {
    let padded = format!("  {k} ");
    let b = padded.as_bytes();
    let mut v: Vec<[u8; 3]> = b.windows(3).map(|w| [w[0], w[1], w[2]]).collect();
    v.sort_unstable();
    v.dedup();
    v
}

impl Matcher {
    fn new(d: &GameData) -> Self {
        let mut entries = Vec::new();
        let mut index: HashMap<[u8; 3], Vec<u32>> = HashMap::new();
        let mut tri_count = Vec::new();
        for (text, groups) in &d.mod_texts {
            let k = key(text);
            if k.len() < 4 {
                continue;
            }
            let id = entries.len() as u32;
            let tg = trigrams(&k);
            tri_count.push(tg.len() as u16);
            for t in tg {
                index.entry(t).or_default().push(id);
            }
            entries.push((k, text.replace('\n', " "), groups.clone()));
        }
        Self { entries, trigrams: index, tri_count }
    }

    /// Best stat text for an OCR line: (index, score).
    fn best(&self, ocr_key: &str, n_values: usize) -> Option<(usize, f32)> {
        let tg = trigrams(ocr_key);
        if tg.is_empty() {
            return None;
        }
        let mut shared = vec![0u16; self.entries.len()];
        for t in &tg {
            for &id in self.trigrams.get(t).map(Vec::as_slice).unwrap_or_default() {
                shared[id as usize] += 1;
            }
        }
        let mut cands: Vec<(usize, f32)> = shared
            .iter()
            .enumerate()
            .filter(|(_, s)| **s > 0)
            .map(|(i, s)| (i, 2.0 * *s as f32 / (tg.len() as f32 + self.tri_count[i] as f32)))
            .collect();
        cands.sort_by(|a, b| b.1.total_cmp(&a.1));
        cands.truncate(40);
        cands
            .into_iter()
            .map(|(i, _)| {
                let k = &self.entries[i].0;
                let mut s = similarity(ocr_key, k);
                // The number of values must agree with the number of `#` in the template.
                let holes = k.matches('#').count();
                if holes != n_values {
                    s -= 0.08 * holes.abs_diff(n_values) as f32;
                }
                (i, s)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
    }
}

/// Values in OCR order, then the trade template with `#` filled in.
fn fill(template: &str, values: &[f64]) -> String {
    let mut out = String::new();
    let mut vi = values.iter();
    for ch in template.chars() {
        if ch == '#' {
            match vi.next() {
                Some(v) if v.fract() == 0.0 => out.push_str(&format!("{}", *v as i64)),
                Some(v) => out.push_str(&format!("{v}")),
                None => out.push('#'),
            }
        } else {
            out.push(ch);
        }
    }
    // Text in the trade data is sign-less for PoE 2 and for negative rolls; the clipboard keeps
    // what the game prints, which the clipboard parser handles the same way.
    out
}

// ---------------------------------------------------------------- colours

/// Rarity guessed from the header text colour.
pub fn rarity_from_color(c: [u8; 3]) -> Option<Rarity> {
    let [r, g, b] = c.map(|v| v as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let v = max;
    let s = if max > 0.0 { (max - min) / max } else { 0.0 };
    if v < 0.25 {
        return None;
    }
    if s < 0.18 {
        return Some(Rarity::Normal);
    }
    let h = if max == min {
        0.0
    } else if max == r {
        60.0 * (((g - b) / (max - min)).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / (max - min) + 2.0)
    } else {
        60.0 * ((r - g) / (max - min) + 4.0)
    };
    Some(match h {
        h if (200.0..290.0).contains(&h) => Rarity::Magic,
        h if (45.0..75.0).contains(&h) && s > 0.3 => Rarity::Rare,
        h if (10.0..45.0).contains(&h) && s > 0.45 => Rarity::Unique,
        // On a TV photo the unique brown-orange washes out towards a pinkish orange.
        h if h < 25.0 && s > 0.25 => Rarity::Unique,
        h if (10.0..60.0).contains(&h) => Rarity::Currency,
        h if (150.0..200.0).contains(&h) => Rarity::Gem,
        _ => return None,
    })
}

/// Share of blue, red, green in a colour (brightness-independent).
fn chroma(c: [u8; 3]) -> [f32; 3] {
    let sum = (c[0] as f32 + c[1] as f32 + c[2] as f32).max(1.0);
    [c[0] as f32 / sum, c[1] as f32 / sum, c[2] as f32 / sum]
}

/// Mod kind from line colour, relative to the item's typical mod colour (the explicit blue):
/// crafted/enchant are a paler, greener blue; fractured is gold. Relative, because photos shift
/// absolute colours (white balance, glare, TV picture modes).
fn kind_from_color(c: [u8; 3], typical: [f32; 3]) -> Option<&'static str> {
    let [r, g, b] = chroma(c);
    let less_blue = typical[2] - b;
    if less_blue > 0.07 && r >= g {
        Some("fractured")
    } else if less_blue > 0.025 && g > r {
        Some("crafted")
    } else {
        None
    }
}

// ---------------------------------------------------------------- properties and flags

/// Property labels as the clipboard writes them.
const PROPS: &[&str] = &[
    "Quality", "Item Level", "Armour", "Evasion Rating", "Evasion", "Energy Shield", "Ward",
    "Chance to Block", "Block chance", "Physical Damage", "Elemental Damage", "Chaos Damage",
    "Attacks per Second", "Critical Strike Chance", "Critical Hit Chance", "Stack Size", "Level",
    "Spirit", "Reload Time", "Charm Slots", "Map Tier", "Item Quantity", "Item Rarity",
];
const SKIP_PREFIXES: &[&str] = &["Requires", "Requirements", "Sockets", "Experience", "Limited to", "Grants Skill"];
const FLAGS: &[&str] = &["Corrupted", "Unidentified", "Mirrored", "Split", "Unmodifiable"];

static LABEL_VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z' ]+?)\s*[:;.]?\s*([+\-]?\d.*)$").unwrap());

fn as_property(line: &str) -> Option<String> {
    let c = LABEL_VALUE.captures(line)?;
    let label = c[1].trim();
    let (best, score) = PROPS
        .iter()
        .map(|p| (*p, similarity(&key(label), &key(p))))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    (score >= 0.8).then(|| format!("{best}: {}", c[2].trim()))
}

fn as_flag(line: &str) -> Option<&'static str> {
    FLAGS.iter().copied().find(|f| similarity(&key(line), &key(f)) >= 0.75)
}

fn is_skipped(line: &str) -> bool {
    let k = key(line);
    SKIP_PREFIXES.iter().any(|p| {
        let pk = key(p);
        k.len() >= pk.len() && similarity(&k[..pk.len()], &pk) >= 0.75
    })
}

// ---------------------------------------------------------------- layout

struct Row {
    text: String,
    top: f32,
    bottom: f32,
    color: Option<[u8; 3]>,
}

/// Sort lines top to bottom, merge boxes on the same row, split into sections at large gaps.
fn layout(lines: &[OcrLine]) -> Vec<Vec<Row>> {
    let mut ls: Vec<&OcrLine> = lines.iter().filter(|l| !l.text.trim().is_empty()).collect();
    ls.sort_by(|a, b| a.top.total_cmp(&b.top));
    let mut rows: Vec<(Row, Vec<&OcrLine>)> = Vec::new();
    for l in ls {
        let h = l.bottom - l.top;
        if let Some((row, parts)) = rows.last_mut() {
            let overlap = row.bottom.min(l.bottom) - row.top.max(l.top);
            // Same row only if the boxes sit side by side. On a tilted photo OCR can return a
            // box much taller than its line that overlaps the lines above and below.
            let side_by_side = parts.iter().all(|p| l.left >= p.right - 0.5 * h || l.right <= p.left + 0.5 * h);
            if overlap > 0.5 * h.min(row.bottom - row.top) && side_by_side {
                parts.push(l);
                row.top = row.top.min(l.top);
                row.bottom = row.bottom.max(l.bottom);
                continue;
            }
        }
        rows.push((Row { text: String::new(), top: l.top, bottom: l.bottom, color: l.color }, vec![l]));
    }
    let rows: Vec<Row> = rows
        .into_iter()
        .map(|(mut r, mut parts)| {
            parts.sort_by(|a, b| a.left.total_cmp(&b.left));
            r.text = parts.iter().map(|p| p.text.trim()).collect::<Vec<_>>().join(" ");
            // The first box's colour; for "Label: value" rows the label comes first.
            r.color = parts.iter().find_map(|p| p.color);
            r
        })
        .collect();
    if rows.is_empty() {
        return vec![];
    }
    // Dividers add space between lines. Measure line pitch centre to centre (box heights vary
    // with underlines, icons and OCR padding, so gaps between boxes don't work: inside a section
    // they're 4-10 px on one screenshot while dividers are only 8-12 px on a photo). Lines inside
    // a section are evenly spaced, so normal spacing is the smallest pitch (ignoring near-overlaps)
    // and a divider is a pitch clearly above it. If every pitch is about the same, every section
    // is one line long or there are no dividers: decide by size.
    let mut hs: Vec<f32> = rows.iter().map(|r| r.bottom - r.top).collect();
    hs.sort_by(f32::total_cmp);
    let h = hs[hs.len() / 2].max(1.0);
    let centre = |r: &Row| (r.top + r.bottom) / 2.0;
    // Pitch between row i and i+1; near-overlaps (OCR split one line) count as no break.
    let pitches: Vec<f32> = rows.windows(2).map(|w| centre(&w[1]) - centre(&w[0])).collect();
    let valid: Vec<f32> = pitches.iter().copied().filter(|p| *p >= 0.6 * h).collect();
    let min = valid.iter().copied().fold(f32::MAX, f32::min);
    let max = valid.iter().copied().fold(0.0, f32::max);
    let uniform = valid.is_empty() || max < 1.2 * min;
    let is_break = |i: usize| -> bool {
        let p = pitches[i];
        if p < 0.6 * h {
            return false;
        }
        if uniform {
            return p > 1.8 * h;
        }
        // Perspective makes spacing grow across a photo: compare with nearby pitches, capped
        // for runs of one-line sections (where every nearby pitch is a divider).
        let lo = i.saturating_sub(3);
        let hi = (i + 4).min(pitches.len());
        let local = pitches[lo..hi].iter().copied().filter(|p| *p >= 0.6 * h).fold(f32::MAX, f32::min);
        p > 1.2 * local.min(1.3 * min)
    };
    let mut sections: Vec<Vec<Row>> = vec![];
    for (i, r) in rows.into_iter().enumerate() {
        if i == 0 || is_break(i - 1) {
            sections.push(vec![]);
        }
        sections.last_mut().unwrap().push(r);
    }
    sections
}

// ---------------------------------------------------------------- header

struct Header {
    rarity: Rarity,
    name: Option<String>,
    base: String,
    lines_used: usize,
    source: &'static str,
    confidence: f32,
}

fn best_base<'a>(d: &'a GameData, line: &str) -> Option<(&'a str, f32)> {
    let k = key(line);
    d.bases
        .iter()
        .map(|b| (b.as_str(), similarity(&k, &key(b))))
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

fn best_base_inside<'a>(d: &'a GameData, line: &str) -> Option<(&'a str, f32)> {
    let k = key(line);
    // Longest first, so "Ruby Ring" wins over "Ring" at equal score.
    let mut best: Option<(&str, f32)> = None;
    for b in &d.bases {
        let bk = key(b);
        if bk.len() < 4 || bk.len() > k.len() + 2 {
            continue;
        }
        let s = substring_similarity(&bk, &k);
        if best.is_none_or(|(_, bs)| s > bs + 1e-6) {
            best = Some((b.as_str(), s));
        }
    }
    best
}

fn best_unique<'a>(d: &'a GameData, line: &str, base: &str) -> Option<(&'a str, f32)> {
    let k = key(&fold_accents(line));
    d.uniques
        .iter()
        .filter(|(_, b)| b == base)
        // Banner ornaments read as letters can be glued to the name ("7ANUSSILVERTHORNE S"),
        // so also look for the name inside the line.
        .map(|(n, _)| {
            let nk = key(n);
            let inside = if k.len() > nk.len() { substring_similarity(&nk, &k) - 0.05 } else { 0.0 };
            (n.as_str(), similarity(&k, &nk).max(inside))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// Rare names are two words; tokens of one or two characters at either end are banner
/// ornaments read as letters ("NA BROOD BANDA" keeps "BROOD BANDA").
fn trim_junk(name: &str) -> String {
    let mut t: Vec<&str> = name.split_whitespace().collect();
    while t.len() > 2 && t.first().is_some_and(|w| w.chars().count() <= 2) {
        t.remove(0);
    }
    while t.len() > 2 && t.last().is_some_and(|w| w.chars().count() <= 2) {
        t.pop();
    }
    t.join(" ")
}

fn read_header(d: &GameData, rows: &[&Row]) -> Option<Header> {
    let l0 = rows.first()?;
    let color = l0.color.and_then(rarity_from_color);
    let base_of = |b: &str| match d.base_group.get(b).map(String::as_str) {
        Some("currency") => Rarity::Currency,
        Some("gem") => Rarity::Gem,
        Some("card") => Rarity::DivinationCard,
        _ => Rarity::Normal,
    };

    // Two-line header: name + base (rare or unique).
    if let Some(l1) = rows.get(1) {
        if let Some((b, bs)) = best_base(d, &l1.text).filter(|(_, s)| *s >= 0.8) {
            // A unique name only counts if that unique really uses this base.
            let uniq = best_unique(d, &l0.text, b).filter(|(_, s)| *s >= 0.8);
            let (rarity, name, source) = match (color, uniq) {
                (Some(Rarity::Rare), _) => (Rarity::Rare, trim_junk(&l0.text), "colour"),
                (_, Some((n, _))) => (Rarity::Unique, n.to_string(), "unique name"),
                (Some(Rarity::Unique), None) => (Rarity::Unique, trim_junk(&l0.text), "colour"),
                _ => (Rarity::Rare, trim_junk(&l0.text), "header shape"),
            };
            return Some(Header { rarity, name: Some(name), base: b.to_string(), lines_used: 2, source, confidence: bs });
        }
    }
    // One-line header: normal / currency / gem (exact base) or magic (base inside affixes).
    let (b, bs) = best_base(d, &l0.text)?;
    if bs >= 0.85 && color != Some(Rarity::Magic) {
        let r = base_of(b);
        let source = if color.is_some() { "base match + colour" } else { "base match" };
        return Some(Header { rarity: r, name: None, base: b.to_string(), lines_used: 1, source, confidence: bs });
    }
    let (b, bs) = best_base_inside(d, &l0.text)?;
    (bs >= 0.8).then(|| Header {
        rarity: Rarity::Magic,
        name: None,
        base: b.to_string(),
        lines_used: 1,
        source: if color == Some(Rarity::Magic) { "colour" } else { "header shape" },
        confidence: bs,
    })
}

// ---------------------------------------------------------------- main entry

/// Keep only the tooltip's lines. Screens and photos include other UI (stash tabs, labels, chat)
/// that OCR reads too, sometimes on the same row as a tooltip line. Tooltip text is always
/// centred, so: find the header (ideally name + base stacked on the same centre line, with many
/// centred lines below), then keep lines centred on it until the text stops.
fn isolate(lines: &[OcrLine], d: &GameData) -> Vec<OcrLine> {
    let mut ls: Vec<&OcrLine> = lines.iter().filter(|l| !l.text.trim().is_empty()).collect();
    if ls.len() < 2 {
        return ls.into_iter().cloned().collect();
    }
    ls.sort_by(|a, b| a.top.total_cmp(&b.top));
    let mut hs: Vec<f32> = ls.iter().map(|l| l.bottom - l.top).collect();
    hs.sort_by(f32::total_cmp);
    let h = hs[hs.len() / 2].max(1.0);
    let cx = |l: &OcrLine| (l.left + l.right) / 2.0;
    let row = |l: &OcrLine| Row { text: l.text.clone(), top: l.top, bottom: l.bottom, color: l.color };
    let aligned_below = |i: usize| {
        ls.iter()
            .filter(|l| l.top > ls[i].bottom && l.top < ls[i].bottom + 25.0 * h && (cx(l) - cx(ls[i])).abs() < 0.7 * h)
            .count() as f32
    };
    let mut best: Option<(f32, usize)> = None;
    for i in 0..ls.len() {
        // The line right below on the same centre line, if any (base under name).
        let next = ls
            .iter()
            // Banner ornaments read as letters can widen the name's box, so be lenient.
            .filter(|l| l.top > ls[i].top + 0.5 * h && l.top - ls[i].bottom < 1.5 * h && (cx(l) - cx(ls[i])).abs() < 1.5 * h)
            .min_by(|a, b| a.top.total_cmp(&b.top));
        let r0 = row(ls[i]);
        let r1 = next.map(|n| row(n));
        let rows: Vec<&Row> = std::iter::once(&r0).chain(r1.as_ref()).collect();
        let Some(hd) = read_header(d, &rows) else { continue };
        // Count centred lines below the base line (the cleanest header line).
        let base_i = match (hd.lines_used, next) {
            (2, Some(n)) => ls.iter().position(|l| std::ptr::eq(*l, *n)).unwrap_or(i),
            _ => i,
        };
        let score = if hd.lines_used == 2 { 3.0 } else { 0.0 } + aligned_below(base_i).min(12.0) + hd.confidence;
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, i));
        }
    }
    let Some((_, a)) = best else { return ls.into_iter().cloned().collect() };
    // Follow the centre line down from the header. In a skewed photo it drifts a little per
    // line, so compare each line with the centre of the last kept line, not the header's.
    let atop = ls[a].top;
    let mut centre = cx(ls[a]);
    let mut out: Vec<OcrLine> = vec![];
    let mut last_bottom = ls[a].bottom;
    for l in &ls {
        if l.top < atop - 0.3 * h || (cx(l) - centre).abs() > 1.5 * h {
            continue;
        }
        if l.top - last_bottom > 6.0 * h {
            break; // past the bottom of the tooltip (allows for a line or two OCR missed)
        }
        last_bottom = last_bottom.max(l.bottom);
        centre = 0.5 * centre + 0.5 * cx(l);
        out.push((*l).clone());
    }
    out
}

pub fn parse(lines: &[OcrLine], d: &GameData) -> Option<ScreenItem> {
    let m = d.matcher.get_or_init(|| Matcher::new(d));
    let lines = isolate(lines, d);
    let sections = layout(&lines);
    let all: Vec<&Row> = sections.iter().flatten().collect();

    // The header may be preceded by junk (window chrome, other UI). Take the first row from
    // which a header can be read.
    let (start, header) = (0..all.len().min(4)).find_map(|i| read_header(d, &all[i..]).map(|h| (i, h)))?;
    let header_rows = start + header.lines_used;

    let mut rep = Report {
        name: header.name.clone(),
        base: header.base.clone(),
        rarity: format!("{:?}", header.rarity),
        rarity_source: header.source.into(),
        base_confidence: header.confidence,
        ..Default::default()
    };

    // Walk the remaining rows section by section.
    struct Line {
        text: String,
        color: Option<[u8; 3]>,
        /// Stat groups the matched text exists in (empty for non-mod lines).
        groups: Vec<String>,
    }
    let mut out_sections: Vec<Vec<Line>> = vec![];
    let mut mod_sections: Vec<usize> = vec![];
    let mut seen = 0usize;
    for sec in &sections {
        let rows: Vec<&Row> = sec
            .iter()
            .filter(|_| {
                seen += 1;
                seen > header_rows
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        let mut out = vec![];
        let mut has_mod = false;
        let mut i = 0;
        while i < rows.len() {
            let raw = clean_ocr(&rows[i].text);
            if let Some(p) = as_property(&raw) {
                rep.properties.push(p.clone());
                out.push(Line { text: p, color: None, groups: vec![] });
                i += 1;
                continue;
            }
            if let Some(f) = as_flag(&raw) {
                out.push(Line { text: f.to_string(), color: None, groups: vec![] });
                i += 1;
                continue;
            }
            if is_skipped(&raw) {
                i += 1;
                continue;
            }
            let score_line = |t: &str| {
                readings(t)
                    .into_iter()
                    .filter_map(|r| {
                        let values: Vec<f64> = NUM.find_iter(&r).filter_map(|x| x.as_str().parse().ok()).collect();
                        m.best(&key(&r), values.len()).map(|(idx, s)| (idx, s, values))
                    })
                    .max_by(|a, b| a.1.total_cmp(&b.1))
            };
            let single = score_line(&raw);
            // A long mod may wrap onto the next row; keep the join if it scores clearly better.
            let joined = rows.get(i + 1).map(|r| format!("{raw} {}", clean_ocr(&r.text))).and_then(|j| {
                let s = score_line(&j)?;
                Some((j, s))
            });
            let s1 = single.as_ref().map_or(0.0, |s| s.1);
            let (ocr, best, used) = match joined {
                Some((j, sj)) if sj.1 > s1 + 0.1 && sj.1 >= MOD_THRESHOLD => (j, Some(sj), 2),
                _ => (raw.clone(), single, 1),
            };
            match best {
                Some((idx, score, mut values)) if score >= MOD_THRESHOLD => {
                    let (_, template, groups) = &m.entries[idx];
                    // Small-caps "%" read as "6" ("+24%" -> "+246"): the template wants a "%"
                    // after this value but OCR has none, so the trailing 6 is the percent sign.
                    let pct_slots: Vec<bool> = template.split('#').skip(1).map(|rest| rest.starts_with('%')).collect();
                    let read = readings(&ocr).into_iter().max_by_key(|r| NUM.find_iter(r).count() == values.len()).unwrap_or_default();
                    for (vi, mt) in NUM.find_iter(&read).enumerate() {
                        let has_pct = read[mt.end()..].starts_with('%');
                        if pct_slots.get(vi) == Some(&true) && !has_pct && mt.as_str().len() >= 2 && mt.as_str().ends_with('6') {
                            if let Some(v) = values.get_mut(vi) {
                                *v = mt.as_str()[..mt.as_str().len() - 1].parse().unwrap_or(*v);
                            }
                        }
                    }
                    // OCR reads a leading "+" as "4" ("+24%" -> "424%").
                    if template.starts_with("+#") && ocr.starts_with('4') && ocr.len() > 2 {
                        if let Some(v) = values.first_mut().filter(|v| **v >= 40.0) {
                            let s = format!("{v}");
                            *v = s[1..].parse().unwrap_or(*v);
                        }
                    }
                    let text = fill(template, &values);
                    // Layout picks implicit vs explicit later; keep the groups so a stat that
                    // can't be that kind (rune, enchant, implicit-only…) gets one it can be.
                    rep.mods.push(ModMatch { ocr: ocr.clone(), text: text.clone(), kind: String::new(), confidence: score });
                    out.push(Line { text, color: rows[i].color, groups: groups.clone() });
                    has_mod = true;
                }
                other => rep.unmatched.push((ocr.clone(), other.map_or(0.0, |o| o.1))),
            }
            i += used;
        }
        if has_mod {
            mod_sections.push(out_sections.len());
        }
        out_sections.push(out);
    }

    // Layout decides implicit vs explicit: with several mod sections, all but the last are
    // implicits (or enchants); a lone mod section on a normal item is implicit.
    // Typical mod colour: median blue share over all mod lines.
    let mut blues: Vec<[f32; 3]> = out_sections.iter().flatten().filter(|l| !l.groups.is_empty()).filter_map(|l| l.color.map(chroma)).collect();
    blues.sort_by(|a, b| a[2].total_cmp(&b[2]));
    let typical = blues.get(blues.len() / 2).copied();
    let last_mod = mod_sections.last().copied();
    let normal = header.rarity == Rarity::Normal;
    // PoE 2 armour (gloves, boots, helmets, body armour, shields, foci) has no implicit mods, so a
    // mod section above the explicits there is socketed runes / soul cores.
    let no_implicits = d.game == crate::config::Game::Poe2 && d.base_group.get(&header.base).is_some_and(|g| g == "armour");
    let mut mi = 0;
    let mut text = String::new();
    text.push_str(&format!(
        "Rarity: {}\n",
        match header.rarity {
            Rarity::Normal => "Normal",
            Rarity::Magic => "Magic",
            Rarity::Rare => "Rare",
            Rarity::Unique => "Unique",
            Rarity::Currency => "Currency",
            Rarity::Gem => "Gem",
            Rarity::DivinationCard => "Divination Card",
            Rarity::Other => "Other",
        }
    ));
    if let Some(n) = &header.name {
        text.push_str(&format!("{n}\n"));
    }
    text.push_str(&format!("{}\n", header.base));
    for (si, sec) in out_sections.iter().enumerate() {
        if sec.is_empty() {
            continue;
        }
        text.push_str("--------\n");
        let is_mod_sec = mod_sections.contains(&si);
        let implicit_sec = is_mod_sec && (Some(si) != last_mod || normal);
        for l in sec {
            let is_mod = is_mod_sec && !PROPS.iter().any(|p| l.text.starts_with(&format!("{p}: "))) && !FLAGS.contains(&l.text.as_str());
            let guess = match (implicit_sec, no_implicits) {
                (true, true) => "rune",
                (true, false) => "implicit",
                _ => "explicit",
            };
            let kind: Option<&str> = if !is_mod {
                None
            } else if let Some(g) = l
                .color
                .zip(typical)
                .and_then(|(c, t)| kind_from_color(c, t))
                .filter(|g| l.groups.iter().any(|x| x == g))
            {
                Some(g)
            } else if l.groups.iter().any(|g| g == guess) {
                Some(guess)
            } else if let Some(k) = implicit_sec
                .then(|| ["rune", "enchant"].into_iter().find(|k| l.groups.iter().any(|g| g == k)))
                .flatten()
            {
                // A section above the explicits whose stat can't be an implicit: on PoE 2 that's
                // a socketed rune (or an enchant), not an explicit.
                Some(k)
            } else {
                ["explicit", "implicit", "rune", "enchant", "crafted", "fractured", "desecrated"]
                    .into_iter()
                    .find(|k| l.groups.iter().any(|g| g == k))
                    .or(Some(guess))
            };
            match kind {
                Some(k) => {
                    let suffix = if k == "explicit" { String::new() } else { format!(" ({k})") };
                    text.push_str(&format!("{}{suffix}\n", l.text));
                    if let Some(mm) = rep.mods.get_mut(mi) {
                        mm.kind = k.to_string();
                    }
                    mi += 1;
                }
                None => text.push_str(&format!("{}\n", l.text)),
            }
        }
    }
    rep.clipboard = text.clone();
    let item = item::parse(&text)?;
    Some(ScreenItem { item, report: rep })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::fixtures::{POE1, POE2};

    fn lines(texts: &[&str]) -> Vec<OcrLine> {
        // Rows 20px high, 4px apart; an empty string marks a divider (extra 20px gap).
        let mut y = 0.0;
        let mut out = vec![];
        for t in texts {
            if t.is_empty() {
                y += 20.0;
                continue;
            }
            out.push(OcrLine { text: t.to_string(), left: 10.0, top: y, right: 300.0, bottom: y + 20.0, color: None });
            y += 24.0;
        }
        out
    }

    #[test]
    fn undoes_to_read_as_ten() {
        assert_eq!(readings("+21 10 STRENGTH")[1], "+21 to STRENGTH");
        assert_eq!(readings("+5210 MAXIMUM Lire")[1], "+52 to MAXIMUM Lire");
        assert_eq!(readings("ADDS 3 10 7 FIRE DAMAGE")[1], "ADDS 3 to 7 FIRE DAMAGE");
        assert_eq!(readings("+10% to Fire Resistance").len(), 1);
    }

    #[test]
    fn percent_read_as_six() {
        let l = lines(&["X", "Sapphire Ring", "", "+246 TO COLD RESISTANCE", "+16% to Fire Resistance"]);
        let s = parse(&l, &POE1).unwrap();
        let v: Vec<f64> = s.item.mods.iter().map(|m| m.values[0]).collect();
        assert_eq!(v, vec![24.0, 16.0]);
    }

    #[test]
    fn folds_accents_and_trims_names() {
        assert_eq!(clean_ocr("100% INCREAŞED PARRY DAMAGE"), "100% INCREASED PARRY DAMAGE");
        assert_eq!(clean_ocr("CHANCE TỚ FIRE"), "CHANCE TO FIRE");
        assert_eq!(trim_junk("NA BROOD BANDA"), "BROOD BANDA");
        assert_eq!(trim_junk("Brood Band"), "Brood Band");
    }

    #[test]
    fn cleans_ocr_digits() {
        assert_eq!(clean_ocr("+1O% to Fire Resistance"), "+10% to Fire Resistance");
        assert_eq!(clean_ocr("Adds l2 to 2S Fire Damage"), "Adds 12 to 25 Fire Damage");
        assert_eq!(clean_ocr("Item  Level: 8l"), "Item Level: 81");
        assert_eq!(clean_ocr("Sockets"), "Sockets");
    }

    #[test]
    fn parses_noisy_rare_boots() {
        let l = lines(&[
            "Havoc Stride", "Slink Boots", "",
            "Evasion Rating: 26O", "",
            "Requires Level 69, 82 Dex", "",
            "Item Level: 82", "",
            "+52 to maxirnum Life", "25% increased Movernent Speed", "+31% to Cold Resistanoe",
            "+24% to Lightning Resistance", "+18 to Dexterlty",
        ]);
        let s = parse(&l, &POE1).unwrap();
        let it = &s.item;
        assert_eq!(it.rarity, Rarity::Rare);
        assert_eq!(it.base, "Slink Boots");
        assert_eq!(it.item_level, Some(82));
        assert_eq!(it.props, vec![("ev", 260.0)]);
        let t: Vec<&str> = it.mods.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(t, vec![
            "+52 to maximum Life", "25% increased Movement Speed", "+31% to Cold Resistance",
            "+24% to Lightning Resistance", "+18 to Dexterity",
        ]);
        assert!(s.report.unmatched.is_empty(), "{:?}", s.report.unmatched);
    }

    #[test]
    fn implicit_section_and_magic_base() {
        let l = lines(&["Glinting Ruby Ring of the Fox", "", "+29% to Fire Resistance", "", "+12 to Dexterity", "", "Corrupted"]);
        let s = parse(&l, &POE1).unwrap();
        assert_eq!(s.item.rarity, Rarity::Magic);
        assert_eq!(s.item.base, "Ruby Ring");
        assert!(s.item.corrupted);
        let k: Vec<_> = s.item.mods.iter().map(|m| (m.kind.trade_type(), m.template.as_str())).collect();
        assert_eq!(k, vec![("implicit", "+#% to Fire Resistance"), ("explicit", "+# to Dexterity")]);
    }

    #[test]
    fn unique_and_currency() {
        let s = parse(&lines(&["Tabula Rasa", "Simple Robe"]), &POE1).unwrap();
        assert_eq!(s.item.rarity, Rarity::Unique);
        let s = parse(&lines(&["Divine Orb", "", "Stack Size: 3/20"]), &POE1).unwrap();
        assert_eq!(s.item.rarity, Rarity::Currency);
        assert_eq!(s.item.stack_size, Some(3));
    }

    #[test]
    fn poe2_sign_less_stats() {
        let l = lines(&["Rune Tread", "Lattice Sandals", "", "+40 to maximum Mana", "+30% to Cold Resistance"]);
        let s = parse(&l, &POE2).unwrap();
        let t: Vec<&str> = s.item.mods.iter().map(|m| m.template.as_str()).collect();
        assert_eq!(t, vec!["# to maximum Mana", "#% to Cold Resistance"]);
    }

    #[test]
    fn rarity_colours() {
        assert_eq!(rarity_from_color([255, 255, 119]), Some(Rarity::Rare));
        assert_eq!(rarity_from_color([175, 96, 37]), Some(Rarity::Unique));
        assert_eq!(rarity_from_color([136, 136, 255]), Some(Rarity::Magic));
        assert_eq!(rarity_from_color([200, 200, 200]), Some(Rarity::Normal));
        assert_eq!(rarity_from_color([170, 158, 130]), Some(Rarity::Currency));
    }
}
