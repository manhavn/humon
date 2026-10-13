# Contributing

Thanks for helping! A few things keep humon healthy:

## Test first

Every change comes with a test, written before the code. Keep logic in plain
functions so it can be tested without a window; UI code should mostly wire
those functions together.

```sh
cargo test --release
```

The live tests talk to a real MongoDB. Point them at any test server (the data is
created by the script) and include them:

```sh
mongosh "$URI" scripts/seed-test-db.js
HUMON_TEST_URI="$URI" cargo test --release -- --include-ignored
```

`--include-ignored` also runs `keychain_round_trip`, which writes and deletes a test
item in your real Keychain (under `dev.muzaffer.humon.test`), and a test that asks
GitHub for the latest release. CI skips both.

## Before you push

```sh
cargo fmt
cargo clippy --release --all-targets -- -D warnings
```

CI runs the same checks, plus the live tests.

## Versions and releases

humon follows [Semantic Versioning](https://semver.org/); the version lives in
`Cargo.toml` only. Note changes under `## [Unreleased]` in `CHANGELOG.md` as you go.

To release:

1. Rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD` (and add a fresh
   `## [Unreleased]` above it).
2. Run `scripts/release.sh X.Y.Z`: it sets the version, runs the tests, commits
   and tags.
3. `git push origin main vX.Y.Z`. The release workflow builds a universal macOS
   app and publishes it with the changelog section as the notes.

The workflow refuses a tag that doesn't match `Cargo.toml`, and a test fails if the
current version has no changelog section.

## Translations

Adding a language is one file in `locales/`, no code changes: copy `locales/en.yml`
to `locales/<code>.yml` and translate the values, keeping `%{...}` as it is.
`cargo test` tells you if a key or a placeholder is missing.

## Keep it light

humon exists because MongoDB Compass uses too much memory. Before adding a
dependency, check what it costs in binary size and RAM.

## How the code is laid out

| Path | What it does |
| --- | --- |
| `src/app/` | The window: sidebar, connections, tab bar, quick open, settings, restoring tabs |
| `src/tab/` | A collection tab: queries and paging, tree/table/JSON/schema views, editor, history, export, code, indexes and explain panels, status bar |
| `src/console/` | The shell: QuickJS engine, mongosh-like API (`prelude.js`), history, completion, tab view |
| `src/ui.rs` | Shared measures and building blocks: bars, panels, buttons |
| `src/mongo.rs` | Talking to MongoDB (runs on tokio); `CollectionRef` holds the collection operations |
| `src/query.rs` | Mongo shell syntax → BSON |
| `src/shell.rs` | Extended JSON → shell text |
| `src/highlight/` | Syntax colors: a pure lexer, and its hookup to the code editor |
| `src/complete/` | Completion and warnings for MQL: in the query boxes, in pipelines, and in the shell |
| `src/schema/` | Collection schemas: sampling, searching for what the sample missed, how pipeline stages reshape them, the Schema view's rows |
| `src/mql/` | Every operator, stage, expression and accumulator, from MongoDB's specifications |
| `src/codegen.rs` | The query as driver code |
| `src/explain.rs` | Reading `explain` output |
| `src/session.rs` | The open tabs, saved for the next launch |
| `src/smart.rs` | The typing helpers |
| `src/palette.rs` | Quick open's matching and ranking |
| `src/manage.rs` | Creating, renaming and dropping databases and collections |
| `src/version.rs` | Version, release notes, the update check, the people thanked |
| `src/store.rs` | Saved connections, with passwords in the Keychain |
| `src/history.rs`, `src/settings.rs`, `src/files.rs` | Query history, settings, and the JSON files behind them |
| `scripts/` | Release, app bundle and Homebrew cask scripts, the MQL table generator, test data |
