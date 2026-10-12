<p align="center">
  <img src="assets/logo.png" width="160" alt="humon logo">
</p>

<h1 align="center">humon</h1>

<p align="center">
  A small, fast MongoDB client for your desktop.<br>
  Written in Rust, drawn with <a href="https://www.gpui.rs">gpui</a>. No browser inside.
</p>

---

## Why humon?

It is built to be fast and light:

- **Native, GPU-drawn UI.** No browser and no Electron inside, so it opens instantly
  and stays responsive with large results.
- **Small memory footprint.** Results are streamed, paged and exported straight to
  disk instead of being held in memory.
- **One small app.** A single binary with an embedded JavaScript shell; no Node.js,
  no mongosh, nothing else to install.

## 🤖 Built entirely by AI

Every line of code in this repo was written by an AI ([Claude](https://claude.com/claude-code)),
with a human giving directions, testing, and saying "no, not like that" a lot.
That includes this README. Treat it as an experiment: it works well for me,
but read the code before you trust it with anything important.

## ⚠️ Read this before you use it

**Your connections are saved on your computer in plain text.**

humon keeps saved connections in
`~/Library/Application Support/dev.muzaffer.humon/`. Passwords are split into
`secrets.json`, which only your user can read, but **they are not encrypted**.
Anyone or anything that can read your files can read them.

Please don't save production or other sensitive credentials in humon yet.
Use it with local databases, test clusters, or read-only users. Encrypting
these (macOS Keychain) is the first item on the to-do list below.

## Features

- Save connections and give each one a color; its tabs carry the same color
- Browse connections → databases → collections, with a quick collection search
- Quick open (⌘K or the search box at the top): jump to any collection or open a
  database's shell by typing, e.g. `users`, `shop.ord` or `>_ logs`
- Open collections in tabs (⌘-click or middle-click opens another tab)
- Filter, sort and project with mongo shell syntax:
  `{ _id: ObjectId("…"), createdAt: { $gt: ISODate("2024-01-01") } }`
- Tree, table and JSON views (the JSON view can be selected, copied and searched);
  edit, clone, copy, delete and insert documents
- Query history and favorites for every collection
- Create, rename and drop databases and collections (drops ask you to type the name)
- Export results to JSON or CSV, streamed to disk so big exports stay light
- Helpful typing: brackets close themselves, `Obj` becomes `ObjectId("")`,
  and pasted ids, UUIDs and dates are wrapped for you
- A built-in shell (`>_` on a connection, a database, or a collection's toolbar): mongosh-style JavaScript
  with `db.users.find(...)`, `use`, `show dbs`, `it`, `print` and more, running on
  an embedded QuickJS engine, so no Node.js or mongosh needed
- Status bar with document count and query time
- English and Turkish, and adding a language is one file (see below)

## Install

Download the latest `.dmg` from [Releases](https://github.com/yldrmzffr/humon/releases),
open it and drag humon into Applications. It runs on Apple Silicon and Intel Macs.

The app is not notarized by Apple yet, so the first launch shows a warning. Right-click
humon in Applications and choose **Open** (once), or run:

```sh
xattr -dr com.apple.quarantine /Applications/humon.app
```

## Build from source

You need [Rust](https://rustup.rs) and macOS (Linux and Windows builds are planned).

```sh
cargo run --release
```

The first build takes a few minutes; after that it is quick.

To get a real app with an icon in your Dock:

```sh
./scripts/bundle-macos.sh
open target/release/humon.app
```

## Add your language

All text lives in `locales/`, one file per language.

1. Copy `locales/en.yml` to `locales/<code>.yml` (for example `de.yml`).
2. Translate the values, including `_language_name`. Keep `%{...}` as it is.
3. Run `cargo test --release`. It tells you if a key is missing.

That's it, no code changes. Your language shows up in Settings (⚙).
Pull requests are very welcome.

## Running the tests

humon is built test-first: logic lives in plain functions with unit tests,
storage takes a directory so tests use temp folders, and a live test runs
against a real MongoDB.

```sh
cargo test --release

# Also against a real server on port 27018, after loading the test data:
mongosh mongodb://127.0.0.1:27018 scripts/seed-test-db.js
HUMON_TEST_URI=mongodb://127.0.0.1:27018 cargo test --release -- --include-ignored
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for how changes are made.

## To do

- [ ] **Encrypt saved connections** (store passwords in the macOS Keychain)
- [ ] Tab menu: duplicate, close others (and ⌘D)
- [ ] Keyboard shortcuts: ⌘T, ⌘W, ⌘1–9, ⌘R, ⌘F
- [ ] Remember open tabs and queries between launches
- [ ] Read-only connections, so production can't be changed by accident
- [ ] Aggregation pipeline editor with stage previews
- [ ] Indexes: list, create, drop, and `explain`
- [ ] Edit a single field without opening the whole document
- [ ] Update or delete many documents by filter (with confirmation)
- [ ] Import from JSON / CSV
- [ ] Schema overview: fields, types, how often they appear
- [ ] SSH tunnel and TLS certificate options
- [ ] Notarized macOS releases (no first-launch warning)
- [ ] Linux and Windows builds
- [ ] UI polish: fonts, sizes, spacing
- [ ] UI-level tests with gpui's test harness (needs a second, larger build)

## How it's built

| Path | What it does |
| --- | --- |
| `src/app/` | The window: sidebar, connections, tab bar, settings |
| `src/tab/` | A collection tab: queries and paging, tree and table views, editor, history and export panels, status bar |
| `src/console/` | The shell: QuickJS engine, mongosh-like API (`prelude.js`), history, tab view |
| `src/palette.rs` | Quick open's matching and ranking |
| `src/manage.rs` | Creating, renaming and dropping databases and collections |
| `src/mongo.rs` | Talking to MongoDB (runs on tokio) |
| `src/query.rs` | Mongo shell syntax → BSON |
| `src/shell.rs` | Extended JSON → shell text |
| `src/highlight/` | Syntax colors: a pure lexer, and its hookup to the code editor |
| `src/smart.rs` | The typing helpers |
| `src/store.rs`, `src/history.rs`, `src/settings.rs`, `src/files.rs` | Saved connections, query history, settings, and the JSON files behind them |

## License

[MIT](LICENSE)
