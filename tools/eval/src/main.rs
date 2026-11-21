//! Accuracy harness for the screen parser.
//!
//! For every `<name>.txt` (Ctrl+C ground truth) in the items dir, reads `<name>.png|.jpg` from
//! each image dir, runs Tesseract (stand-in for ML Kit on desktop), samples text colours, runs
//! `screen::parse`, and compares with the clipboard parser's result on the ground truth.
//!
//! A mod counts as correct when it resolves to the same trade stat id, kind and values as a
//! ground-truth mod, i.e. the trade search would be identical.
//!
//! Usage: lootlens-eval <items dir> <image dir>... [--json out.json] [--ocr-cache dir]
//! An image dir can also hold `<name>.ocr.json` (lines exported from ML Kit by the Android app);
//! it is used instead of running Tesseract.

use anyhow::{Context, Result};
use image::{GenericImageView, RgbImage};
use lootlens_core::config::Game;
use lootlens_core::data::GameData;
use lootlens_core::item::{self, Item};
use lootlens_core::screen::{self, OcrLine};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn load_data(game: Game) -> Result<GameData> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core/data");
    let p = if game == Game::Poe1 { "trade" } else { "trade2" };
    GameData::from_json(
        game,
        &std::fs::read_to_string(dir.join(format!("{p}-stats.json")))?,
        &std::fs::read_to_string(dir.join(format!("{p}-items.json")))?,
    )
}

/// Run Tesseract on a contrast-boosted, upscaled copy; return line boxes in original coordinates.
fn tesseract(img: &RgbImage, tmp: &Path) -> Result<Vec<OcrLine>> {
    // Brightest channel (text is coloured on dark), minus a blurred copy (removes glare and
    // uneven backlight), stretched and inverted to dark-on-light for Tesseract.
    let scale = if img.width() < 900 { 3 } else { 1 };
    let gray = image::GrayImage::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y).0;
        image::Luma([p[0].max(p[1]).max(p[2])])
    });
    let bg = image::imageops::fast_blur(&gray, 20.0);
    let mut diff = image::GrayImage::from_fn(gray.width(), gray.height(), |x, y| {
        image::Luma([gray.get_pixel(x, y).0[0].saturating_sub(bg.get_pixel(x, y).0[0])])
    });
    let max = diff.pixels().map(|p| p.0[0]).max().unwrap_or(1).max(1) as u32;
    for p in diff.pixels_mut() {
        p.0[0] = 255 - (p.0[0] as u32 * 255 / max) as u8;
    }
    let big = image::imageops::resize(
        &diff,
        img.width() * scale,
        img.height() * scale,
        image::imageops::FilterType::CatmullRom,
    );
    big.save(tmp)?;
    let tessdata = std::env::var("TESSDATA_PREFIX")
        .unwrap_or_else(|_| format!("{}/.local/share/tessdata", std::env::var("HOME").unwrap_or_default()));
    let out = Command::new("tesseract")
        .arg(tmp)
        .arg("-")
        .args(["--psm", "4", "-l", "eng", "-c", "tessedit_create_tsv=1"])
        .env("TESSDATA_PREFIX", tessdata)
        .output()
        .context("running tesseract")?;
    let tsv = String::from_utf8_lossy(&out.stdout);
    let mut lines: BTreeMap<(u32, u32, u32), (Vec<String>, [f32; 4])> = BTreeMap::new();
    for row in tsv.lines().skip(1) {
        let c: Vec<&str> = row.split('\t').collect();
        if c.len() < 12 || c[0] != "5" || c[11].trim().is_empty() {
            continue;
        }
        let n = |i: usize| c[i].parse::<f32>().unwrap_or(0.0) / scale as f32;
        let k = (c[2].parse().unwrap_or(0), c[3].parse().unwrap_or(0), c[4].parse().unwrap_or(0));
        let (l, t, w, h) = (n(6), n(7), n(8), n(9));
        let e = lines.entry(k).or_insert((vec![], [f32::MAX, f32::MAX, 0.0, 0.0]));
        e.0.push(c[11].to_string());
        e.1 = [e.1[0].min(l), e.1[1].min(t), e.1[2].max(l + w), e.1[3].max(t + h)];
    }
    Ok(lines
        .into_values()
        .map(|(words, b)| OcrLine {
            text: words.join(" "),
            left: b[0],
            top: b[1],
            right: b[2],
            bottom: b[3],
            color: sample_color(img, b),
        })
        .collect())
}

/// Average colour of the brightest quarter of pixels in the box (the text, not the background).
/// Same method as the Android app.
fn sample_color(img: &RgbImage, b: [f32; 4]) -> Option<[u8; 3]> {
    let (x0, y0) = (b[0].max(0.0) as u32, b[1].max(0.0) as u32);
    let (x1, y1) = ((b[2] as u32).min(img.width()), (b[3] as u32).min(img.height()));
    let mut px: Vec<[u8; 3]> = vec![];
    for y in y0..y1 {
        for x in x0..x1 {
            px.push(img.get_pixel(x, y).0);
        }
    }
    if px.is_empty() {
        return None;
    }
    let lum = |p: &[u8; 3]| 30 * p[0] as u32 + 59 * p[1] as u32 + 11 * p[2] as u32;
    px.sort_by_key(|p| std::cmp::Reverse(lum(p)));
    let top = &px[..(px.len() / 4).max(1)];
    let n = top.len() as u32;
    let s = top.iter().fold([0u32; 3], |a, p| [a[0] + p[0] as u32, a[1] + p[1] as u32, a[2] + p[2] as u32]);
    Some([(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8])
}

/// Mod -> (kind, trade stat id, values), resolved the way `trade::prepare` does.
fn resolve(it: &Item, d: &GameData) -> (Vec<(String, String, Vec<i64>)>, usize) {
    let mut out = vec![];
    let mut unresolved = 0;
    for m in &it.mods {
        let g = m.kind.trade_type().to_string();
        let ids = d.stats.get(&(g.clone(), m.template.clone())).or_else(|| {
            (d.game == Game::Poe2).then(|| d.stats.get(&(g.clone(), m.template.replace("+#", "#")))).flatten()
        });
        match ids {
            Some(ids) => {
                let id = ids[0].0.split_once('.').map_or(ids[0].0.clone(), |(_, s)| s.to_string());
                out.push((g, id, m.values.iter().map(|v| (v * 100.0).round() as i64).collect()));
            }
            None => unresolved += 1,
        }
    }
    (out, unresolved)
}

#[derive(Default)]
struct Totals {
    items: usize,
    header_ok: usize,
    gt_mods: usize,
    correct: usize,
    stat_only: usize,
    wrong_kind: usize,
    extra: usize,
    props_ok: usize,
    ilvl_ok: usize,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut json_out = None;
    let mut dirs = vec![];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--json" {
            json_out = it.next().cloned();
        } else {
            dirs.push(PathBuf::from(a));
        }
    }
    let (items_dir, image_dirs) = dirs.split_first().context("usage: lootlens-eval <items> <images>...")?;
    let poe1 = load_data(Game::Poe1)?;
    let poe2 = load_data(Game::Poe2)?;
    let tmp = std::env::temp_dir().join(format!("lootlens-eval-{}.png", std::process::id()));
    let mut report = vec![];

    for dir in image_dirs {
        let mut tot = Totals::default();
        println!("\n== {}", dir.display());
        println!("{:<28} {:>6} {:>8} {:>6} {:>6}  notes", "item", "header", "mods", "extra", "props");
        let mut files: Vec<_> = std::fs::read_dir(items_dir)?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        files.sort();
        for gt_path in files.iter().filter(|p| p.extension().is_some_and(|e| e == "txt")) {
            let stem = gt_path.file_stem().unwrap().to_string_lossy().to_string();
            let d = if stem.contains("poe2") { &poe2 } else { &poe1 };
            let ocr_json = dir.join(format!("{stem}.ocr.json"));
            let lines = if ocr_json.exists() {
                ocr_from_json(&std::fs::read_to_string(&ocr_json)?)?
            } else {
                let Some(img_path) = ["png", "jpg", "jpeg"].iter().map(|e| dir.join(format!("{stem}.{e}"))).find(|p| p.exists())
                else {
                    continue;
                };
                let img = image::open(&img_path)?;
                let _ = img.dimensions();
                tesseract(&img.to_rgb8(), &tmp)?
            };
            let gt = item::parse(&std::fs::read_to_string(gt_path)?).context("ground truth parse")?;
            let parsed = screen::parse(&lines, d);
            if std::env::var("EVAL_DEBUG").is_ok_and(|v| stem.contains(&v)) {
                for l in &lines {
                    eprintln!("  ocr {:>5.0} {:>5.0} x {:>5.0}-{:<5.0} {:?} {:?}", l.top, l.bottom, l.left, l.right, l.color, l.text);
                }
                if let Some(s) = &parsed {
                    eprintln!("{}\n  unmatched {:?}", s.report.clipboard, s.report.unmatched);
                }
            }
            tot.items += 1;
            let (gt_mods, gt_unres) = resolve(&gt, d);
            tot.gt_mods += gt_mods.len();
            let mut notes = vec![];
            if gt_unres > 0 {
                notes.push(format!("{gt_unres} gt lines w/o stat"));
            }
            let Some(s) = parsed else {
                println!("{stem:<28} {:>6} {:>8} {:>6} {:>6}  NO PARSE ({} lines)", "✗", format!("0/{}", gt_mods.len()), "-", "-", lines.len());
                report.push(json!({"set": dir, "item": stem, "parsed": false, "ocr": lines.iter().map(|l| &l.text).collect::<Vec<_>>()}));
                continue;
            };
            // Magic items: the clipboard keeps the full affixed name as "base"; the trade search only
            // needs the base type inside it.
            let base_ok = s.item.base == gt.base || (gt.rarity == item::Rarity::Magic && gt.base.contains(&s.item.base));
            let header = s.item.rarity == gt.rarity && base_ok
                && s.item.name.as_deref().map(str::to_lowercase) == gt.name.as_deref().map(str::to_lowercase);
            if header {
                tot.header_ok += 1;
            } else {
                notes.push(format!("header {:?} {:?}/{:?} (want {:?} {:?}/{:?})", s.item.rarity, s.item.name, s.item.base, gt.rarity, gt.name, gt.base));
            }
            let (got, _) = resolve(&s.item, d);
            let mut remaining = gt_mods.clone();
            let mut correct = 0;
            let mut extra = 0;
            for g in &got {
                if let Some(i) = remaining.iter().position(|r| r == g) {
                    remaining.remove(i);
                    correct += 1;
                } else if let Some(i) = remaining.iter().position(|r| r.1 == g.1 && r.0 == g.0) {
                    remaining.remove(i);
                    tot.stat_only += 1;
                    notes.push(format!("values {:?} want {:?} ({})", g.2, gt_mods.iter().find(|r| r.1 == g.1).map(|r| &r.2), g.1));
                } else if let Some(i) = remaining.iter().position(|r| r.1 == g.1) {
                    remaining.remove(i);
                    tot.wrong_kind += 1;
                    notes.push(format!("kind {} want {} ({})", g.0, gt_mods.iter().find(|r| r.1 == g.1).map_or("", |r| &r.0), g.1));
                } else {
                    extra += 1;
                }
            }
            for r in &remaining {
                if !notes.iter().any(|n| n.contains(&r.1)) {
                    notes.push(format!("missed {} {}", r.0, r.1));
                }
            }
            tot.correct += correct;
            tot.extra += extra;
            let props_ok = s.item.props == gt.props
                && s.item.weapon.as_ref().map(|w| (w.phys, w.ele, w.aps)) == gt.weapon.as_ref().map(|w| (w.phys, w.ele, w.aps));
            if props_ok {
                tot.props_ok += 1;
            } else {
                notes.push(format!("props {:?} want {:?}", s.item.props, gt.props));
            }
            if s.item.item_level == gt.item_level {
                tot.ilvl_ok += 1;
            }
            println!(
                "{stem:<28} {:>6} {:>8} {:>6} {:>6}  {}",
                if header { "ok" } else { "✗" },
                format!("{correct}/{}", gt_mods.len()),
                extra,
                if props_ok { "ok" } else { "✗" },
                notes.join("; ")
            );
            report.push(json!({
                "set": dir, "item": stem, "parsed": true, "header_ok": header,
                "correct": correct, "gt_mods": gt_mods.len(), "extra": extra,
                "report": s.report,
            }));
        }
        let pct = |a: usize, b: usize| if b == 0 { 0.0 } else { 100.0 * a as f64 / b as f64 };
        println!(
            "TOTAL {} items: header {}/{} | mods correct {}/{} = {:.1}% (right stat, wrong value {}, wrong kind {}) | extra {} | props {}/{} | ilvl {}/{}",
            tot.items, tot.header_ok, tot.items, tot.correct, tot.gt_mods, pct(tot.correct, tot.gt_mods),
            tot.stat_only, tot.wrong_kind, tot.extra, tot.props_ok, tot.items, tot.ilvl_ok, tot.items
        );
    }
    let _ = std::fs::remove_file(&tmp);
    if let Some(p) = json_out {
        std::fs::write(p, serde_json::to_string_pretty(&report)?)?;
    }
    Ok(())
}

fn ocr_from_json(s: &str) -> Result<Vec<OcrLine>> {
    let v: serde_json::Value = serde_json::from_str(s)?;
    Ok(v["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| OcrLine {
            text: l["text"].as_str().unwrap_or_default().to_string(),
            left: l["left"].as_f64().unwrap_or(0.0) as f32,
            top: l["top"].as_f64().unwrap_or(0.0) as f32,
            right: l["right"].as_f64().unwrap_or(0.0) as f32,
            bottom: l["bottom"].as_f64().unwrap_or(0.0) as f32,
            color: l["color"].as_array().map(|c| {
                [0, 1, 2].map(|i| c.get(i).and_then(|x| x.as_u64()).unwrap_or(0) as u8)
            }),
        })
        .collect())
}
