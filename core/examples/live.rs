//! Live check against the trade API: parse a clipboard text, search a realm, print listings.
//! Usage: cargo run -p lootlens-core --example live -- <item.txt> <poe1|poe2> <pc|xbox|sony> [league]
use lootlens_core::config::{Game, Realm, Settings};
use lootlens_core::{data::GameData, item, ninja, trade};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let text = std::fs::read_to_string(&a[0])?;
    let game = if a[1] == "poe2" { Game::Poe2 } else { Game::Poe1 };
    let realm = match a[2].as_str() { "pc" => Realm::Pc, "sony" => Realm::Sony, _ => Realm::Xbox };
    let league = a.get(3).cloned().unwrap_or_else(|| "Standard".into());
    let p = if game == Game::Poe1 { "trade" } else { "trade2" };
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/data");
    let d = GameData::from_json(game, &std::fs::read_to_string(format!("{dir}/{p}-stats.json"))?, &std::fs::read_to_string(format!("{dir}/{p}-items.json"))?)?;
    let it = item::parse(&text).expect("item");
    let cfg = Settings { game, realm, league: league.clone(), ..Settings::default() };
    let spec = trade::prepare(&it, &d, &cfg);
    for f in &spec.filters { println!("[{}] {} min={:?}", if f.enabled { "x" } else { " " }, f.label, f.min); }
    println!("query: {}", trade::build_query(&spec));
    let r = trade::search(&spec, 5).await?;
    println!("{} matches · {}", r.total, r.url);
    for l in r.listings { println!("  {:>14}  ilvl {:?}  {}", l.price, l.ilvl, l.age); }
    if let Some((m, dt)) = ninja::lookup(game, &league, &it.base, it.stack_size, &lootlens_core::config::user_agent()).await {
        println!("poe.ninja: {m} · {dt}");
    }
    Ok(())
}
