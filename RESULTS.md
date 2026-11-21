# Results — 2026-11-20

## Milestones

| # | Milestone | Status |
|---|---|---|
| 1 | Rust core from loot-lens, tests green | ✅ `cargo test -p lootlens-core`: 14 passed (5 item-parser tests from loot-lens, 9 new) |
| 2 | Screen parser ≥ 90% of mods on 20 test images | ✅ 98.9% on 21 rendered stand-ins; **93.5% on 7 real PoE 2 screenshots** (ML Kit). Short of 20 real ones |
| 3 | Android: image → ML Kit → parser → results | ✅ runs in the emulator (API 36, x86_64) on all 42 test images |
| 4 | Console realms + search + poe.ninja on Android | ✅ real listings shown in the app (PoE 2); PoE 1 Xbox search verified from the desktop |
| 5 | Camera photos, accuracy measured | ✅ first real data: **24/25 mods on 4 phone photos of an Xbox TV** (ML Kit). Camera and zoom confirmed working on a tester's phone |

## Accuracy

21 items (13 PoE 1, 8 PoE 2: rares, magic, uniques, currency), 92 mods that resolve to a trade
stat. A mod counts as correct only if it resolves to the **same trade stat id, kind and values**
as the Ctrl+C ground truth, i.e. the trade search would be identical.

| Test set | OCR | Mods correct | Header (rarity + base) | Properties |
|---|---|---|---|---|
| Rendered "screenshots" | ML Kit (emulator) | **91/92 (98.9%)** | 21/21 | 21/21 |
| Simulated TV photos | ML Kit (emulator) | **91/92 (98.9%)** | 20/21¹ | 21/21 |
| Rendered "screenshots" | Tesseract 5 (desktop) | 91/92 (98.9%) | 21/21 | 21/21 |
| Simulated TV photos | Tesseract 5 (desktop) | 90/92 (97.8%) | 20/21¹ | 19/21 |

¹ The rare name OCR'd as "Dusk L0OP". Rare names aren't searched, so it doesn't affect prices.

Remaining misses:
- **Desecrated mod (PoE 2)**: looks like an explicit on screen, so it's searched as an explicit.
  Fixing it would need a colour or marker difference on screen, if one exists.
- One implicit section in a Tesseract photo that wasn't split from the explicits.

Speed in the emulator: ML Kit takes 0.2–1.2 s per image and the parser takes 5–15 ms. Loading the
trade data and building the stat index happens once per game, at start-up.

The first parser version scored 30%. These fixes brought it to 99%, and they are the kind of
problem to watch for on real photos:
- section dividers: split at the *first* clear jump in line gaps (a line OCR missed leaves an
  outsized gap);
- small caps misreads: "TO"→"10", "+"→"4", "%"→"6";
- crafted/fractured detection by colour *relative to* the item's other mods, since photos shift
  colours;
- unique names accepted only if the unique really uses the matched base ("Doom Fletch" is a rare
  Thicket Bow, not the unique Doomfletch);
- for Tesseract only: subtracting a blurred copy of the image to remove glare (from 30% to 89%).

## Real screenshots (PoE 2, 7 items)

`testdata/real/`: in-game PC screenshots with the UI around the tooltip left in on purpose
(stash tabs, tab labels, chat names), because a photo will include it too. Ground truth was
transcribed from the images; mod kinds checked against the player's Ctrl+C text for 5 of 7 items
(the wand and Brood Paw gloves are still guesses).

| OCR | Mods correct | Header | Properties |
|---|---|---|---|
| ML Kit (emulator) | **29/31 (93.5%)** | 7/7 | 7/7 |
| Tesseract (desktop) | 7/31 (22.6%) | 1/7 | 4/7 |

Remaining ML Kit misses: the desecrated mod again, and "+40 to maximum Energy Shield" where OCR
lost the "+40" (the stat is matched but there's no value).

Tesseract merges the tooltip and the UI beside it into one line, which ruins most rows. ML Kit
keeps them apart. Tesseract only remains as a desktop development tool for the rendered set.

What the real screenshots showed, and the fixes:
- **UI around the tooltip.** OCR merged stash labels into tooltip rows ("+109 TO MAXIMUM MANA Body
  Arnm"), and on one item the stash list came before the header. Fix: tooltip text is always
  centred, so the parser finds the header (name + base on the same centre line, many centred lines
  below) and keeps only lines on that centre line, following it as it drifts in a skewed photo.
  ML Kit went from 54.8% to 90.3%.
- **Uneven line spacing** (underlined keywords, skill icons): a gap inside a section was taken as
  a divider, so explicit mods were labelled implicit. Dividers now need at least 0.6 of a line
  height.
- **Pale mod lines** are crafted mods (essence/crafting bench). The relative-colour check already
  labels them correctly.
- **Summed mods:** the tooltip shows one line when two mods give the same stat ("+109 to maximum
  Mana" = +101 and +8). The trade site indexes the sum, so searching the on-screen line is
  correct.
- **Not on screen:** item level (only in the advanced/Alt view) and sockets (icons on the item art,
  not text). Item level is an opt-in filter on the desktop too. Socketed rune effects do appear as
  text lines and are matched, so the price search is unaffected.

## Phone photos of an Xbox (PoE 2)

`testdata/xbox/`: phone photos of a TV showing the console tooltip, with moiré, tilt, glare and
the console UI (Equipped / Unequip / Inventory / button hints) overlapping it. There's no Ctrl+C
on console, so ground truth is transcribed from the photos.

| Item | ML Kit OCR | Mods correct |
|---|---|---|
| Brood Band, Gold Ring | 1.3 s | 7/7 |
| Havoc Palm, Grim Gloves | 0.8 s | 7/7 |
| Plague Edge, Soaring Spear | 1.2 s | 7/8 |
| Silverthorne, Spiked Buckler (unique) | 1.1 s | 3/3 (+1, see below) |
| **Total** | | **24/25 (96%)** |

ML Kit read these photos almost perfectly; nearly every problem was in the parser:
- **Banner ornaments read as letters** ("NA BROOD BANDA", "7ANUSSILVERTHORNE S"). They widened
  the name's box, and the class line under the base ("Ring") is also a valid base name, so
  the parser first paired "Gold Ring" + "Ring". Fixed by looser name/base centring, measuring
  alignment from the base line, finding a unique's name *inside* the line, and trimming 1–2
  letter tokens from rare names.
- **Section dividers:** in photos OCR boxes are as tall as the line pitch, so box *gaps* can't
  find dividers (normal gaps ~0, dividers 8–12 px, yet 4–10 px gaps inside sections on a
  screenshot). Sections now split on line **pitch** centre to centre, compared with nearby lines
  so perspective (spacing grows towards the bottom of a tilted photo) doesn't split sections.
- **Tall slanted boxes:** on a tilted photo ML Kit can return a box much taller than its line,
  which was merged with the lines above and below. Rows now merge only side-by-side boxes.
  This also fixed the spear's mod wrapped over two lines.
- **Accents** OCR adds to capitals ("INCREAŞED", "TỚ") are folded back to ASCII.
- **Unique header colour** washes out to a pinkish orange on a TV; the colour rule now accepts it.

Remaining misses:
- "+4 to Level of all Projectile Skills": OCR lost the "+4"; the stat is found, without a value.
- **Lone line above the explicits** (rune or implicit?). The trade data settles the spear:
  "40% increased Elemental Damage with Attacks" has no implicit stat in PoE 2, so a line in a
  section above the explicits that can't be an implicit is now read as a rune (else an enchant)
  rather than an explicit. Searching it as an explicit would have filtered out nearly every
  listing. The gloves' "8% increased Attack Speed" exists as both implicit and rune, but PoE 2
  armour has no implicits, and searching it as an implicit found 0 listings (165 without it).
  So on PoE 2 armour a section above the explicits is now read as runes.
- The buckler's "+1" extra is actually a fuzzy-match win. The screen says "10 Stacks of Critical
  Weakness", the trade stat is "# Stack of Critical Weakness", so the exact-text ground truth
  can't resolve it but the parser matches it.

## How much to trust the rendered-set numbers

**The test images are renderings, not game screenshots.** `tools/render_tooltips.py` draws
tooltips from the ground-truth text with PoE's colours, layout and dividers, in Liberation Serif
faked into small caps. The game's font is Fontin SmallCaps. Problems specific to real
screenshots will not show up here: the real font, item art behind the text, sockets, influence
icons, the exact wording of the "Requires" line, and HDR. The "photos" use simulated skew,
moiré, glare, blur and JPEG artefacts, which are gentler than a real phone pointed at a TV.
And the parser was tuned on this same set, so treat 98.9% as an upper bound.

What carries over to real images is the method: matching against the trade site's full stat
list corrects OCR errors well. Every OCR misread in this set that left the line structure intact
was corrected.

## Findings for the open questions

- **PoE 2 console trade:** a single crossplay realm. `/api/trade2/data/leagues` lists only realm
  `poe2`, so the app hides the platform choice for PoE 2.
- **PoE 1 console realms:** `xbox` and `sony` work with `/api/trade/search/<realm>/<league>`
  and `&realm=` on fetch. Console markets are thin: Goldrim on Xbox Covens had 34 listings, 0 of
  them from online sellers (4,346 on PC). So the app **defaults to any seller status** on console,
  with an "Online only" toggle.
- **PoE 1 currency on console:** searching items returns 0 for Divine Orbs (currency trades
  through bulk exchange). poe.ninja only tracks PC, so the app labels its figure "PC economy".
  A proper fix is the `/exchange` endpoint (not built).
- **User-Agent:** `LootingGlass-Android/<version> (proof of concept)`. Each price check makes one search
  and one or two fetches; `Retry-After` is shown on 429.

## Next steps

1. **More real screenshots**, to reach 20, including PoE 1, uniques, currency and items with
   implicits (rings, amulets, belts). Drop them into `testdata/real/` + `testdata/items/`.
2. **A phone and a monitor** for milestone 5: photograph tooltips on the monitor, export OCR
   with batch mode, score them.
3. Then console players' TV photos.
4. Known gaps: PoE 1 currency via bulk exchange; multi-line unique mods and hybrid stats are only
   handled by joining two rows; items with sockets or influence icons are untested.
5. **Filter editing** (added in 0.1.3): tap a filter row to turn it on or off, type min/max,
   switch between "Any <class>" and the exact base, and toggle the corruption filter. The search
   re-runs after a short pause, as on the desktop.
