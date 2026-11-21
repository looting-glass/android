# Looting Glass for Android: notes for agents

Looting Glass for Android price-checks Path of Exile items from a **phone photo of the item
tooltip** (typically a console game on a TV). OCR runs on the phone (ML Kit); parsing and the
trade search are a Rust core shared in spirit with the desktop app,
[Looting Glass](https://github.com/looting-glass/desktop). `README.md` covers the layout,
how the parser works and the dev commands; `RESULTS.md` the measured accuracy. This file holds
the conventions that aren't written down anywhere else.

## Git

- **GitHub Flow**: `main` is always releasable; work happens on short-lived branches named
  after the change type (`feat/…`, `fix/…`, `ci/…`, `docs/…`) and lands through a PR that is
  **squash-merged**.
- Commit messages and PR titles use **Conventional Commits** (`feat(parser): …`, `fix: …`,
  `ci: …`, `docs: …`). The PR title becomes the squash commit on `main` and drives releases.
- Releases come from **release-please**: after each merge it updates a release PR that bumps
  `version.txt` and writes `CHANGELOG.md` (`fix` → patch, `feat` → minor, `docs`/`ci`/`chore` →
  no release). Merging that PR tags `vX.Y.Z`, publishes the GitHub release and builds the signed
  APK. Leave the version and changelog to it.
- The website's Android page shows the latest release's changelog, and a daily deploy picks
  up new releases. After merging a release PR, once the APK is attached, start
  `gh workflow run Pages --repo looting-glass/looting-glass.github.io` to show it right away;
  the website's `AGENTS.md` says what to check.
- Commit and push when the maintainer asks; leave changes uncommitted otherwise. Already-pushed
  history stays as it is.

## Versions and signing

- `version.txt` is the only version. Gradle derives `versionName` and `versionCode`
  (major·10000 + minor·100 + patch) from it, and `scripts/build-rust.sh` passes it to the Rust
  User-Agent. The crates' own `Cargo.toml` versions are not used.
- Release APKs are signed in CI with the keystore from the `LG_KEYSTORE_*` secrets. Local
  release builds fall back to the debug key; a phone can't update a CI-signed install with a
  locally signed APK (or the reverse) without uninstalling. Never commit a keystore.

## Two games, one app

PoE 1 and PoE 2 differ in item text, stat ids, trade API realms and filter groups
(`core/src/item.rs`, `core/src/trade.rs`, `core/src/screen.rs`). Scope every parsing or query
change to the game it was observed in (gate on `Game`), and confirm the other game still scores
as before.

## Verifying changes

- `cargo test -p lootlens-core` for the item and screen parsers.
- **Accuracy**: parser changes must not lower the scores in `RESULTS.md`. Re-run the eval on the
  saved ML Kit OCR for each set (no phone needed):
  `cargo run --release -p lootlens-eval -- testdata/items testdata/mlkit/<screens|photos|real|xbox>`.
  CI runs the same. Update `RESULTS.md` when a score changes.
- New test images: ground truth in `testdata/items/<name>.txt` (the game's Ctrl+C text when
  available), the image in the matching `testdata/` folder, and its ML Kit OCR from the app's
  batch mode (`README.md`) in `testdata/mlkit/`.
- The app itself: build and run it in the emulator (`cd android && ./gradlew installDebug`) and
  check the screen with a test image from the photo picker.
- Repeated runs: the trade API rate-limits quickly; don't loop live searches.

## UI

- Native Jetpack Compose only, no web views. Colours and component looks follow the desktop app
  (`android/.../Theme.kt` copies the desktop's `qml/KdeLook.qml` palette); keep the two in step.
- Phone first: touch targets of at least 48 dp, whole rows as tap targets, nothing that needs
  hover.

## Privacy and screenshots

- The repo is public. Test images can show chat, account or character names and other players'
  names: blur them in the image and remove them from the OCR JSON before committing.
- Show new screenshots to the maintainer before committing them.

## Constraints

- Free and open-source tooling only; GPL-3.0-or-later.
- Fan-made: keep the "not affiliated with or endorsed by Grinding Gear Games" notice in the app
  and README.
- GGG asks API clients to identify themselves: keep the User-Agent, and keep API traffic modest
  (one search and one or two fetches per check).
