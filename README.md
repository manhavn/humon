<p align="center">
  <img src="assets/logo.png" width="140" alt="humon logo">
</p>

<h1 align="center">humon</h1>

<p align="center">
  A small, fast MongoDB client for your desktop.<br>
  Written in Rust, drawn with <a href="https://www.gpui.rs">gpui</a>. No browser inside.
</p>

<p align="center">
  <a href="https://github.com/yldrmzffr/humon/releases/latest"><img src="https://img.shields.io/github/v/release/yldrmzffr/humon" alt="Latest release"></a>
  <a href="https://github.com/yldrmzffr/humon/actions/workflows/ci.yml"><img src="https://github.com/yldrmzffr/humon/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/platform-macOS-lightgrey" alt="Platform: macOS">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/yldrmzffr/humon" alt="License: MIT"></a>
</p>

<p align="center">
  <img src="assets/screenshots/collection.png" width="820" alt="Browsing a collection: the database tree, query bar and colored documents">
</p>

<p align="center">
  <img src="assets/screenshots/shell.png" width="820" alt="The built-in shell: a script on top, selectable results below">
</p>

## Why humon?

It is built to be fast and light:

- **Native, GPU-drawn UI.** No browser and no Electron inside, so it opens instantly
  and stays responsive with large results.
- **Small memory footprint.** Results are streamed, paged and exported straight to
  disk instead of being held in memory.
- **One small app.** A single binary with an embedded JavaScript shell: no Node.js,
  no mongosh, nothing else to install.

## Install

With [Homebrew](https://brew.sh):

```sh
brew install --cask yldrmzffr/tap/humon
```

Or download the latest `.dmg` from
[Releases](https://github.com/yldrmzffr/humon/releases/latest), open it and drag humon
into Applications. It runs on Apple Silicon and Intel Macs (macOS 12 or newer).

The app is not notarized by Apple yet, so the first launch is blocked with a warning.
Open **System Settings → Privacy & Security** and click **Open Anyway** (once), or run:

```sh
xattr -dr com.apple.quarantine /Applications/humon.app
```

humon tells you at the bottom of the sidebar when a newer version is out; with
Homebrew, `brew upgrade` installs it.

## Features

**Connect**
- Any connection string, including Atlas (`mongodb+srv://`) and compressed
  connections (`compressors=zlib`, `zstd`, `snappy`)
- Saved connections, each with a color its tabs share; passwords in the Keychain

**Browse**
- Connections → databases → collections, with a quick collection search
- Quick open (⌘K): jump to any collection or a database's shell by typing,
  e.g. `users`, `shop.ord` or `>_ logs`
- Tabs, including the same collection twice (⌘-click or middle-click); open tabs,
  their queries and shell scripts come back on the next launch

**Query**
- Filter, sort and project in mongo shell syntax:
  `{ _id: ObjectId("…"), createdAt: { $gt: ISODate("2024-01-01") } }`
- Suggestions as you type: field names with their types and how often they appear,
  operators that fit each field (every operator, from MongoDB's own specifications,
  with its docs link), and values like `1`/`-1` for sort
- Warnings as you type: unknown fields (with a "did you mean"), unknown operators,
  values of the wrong type for a field, malformed operator arguments
- Fields are learned from a sample of the collection; then the server is asked for
  documents that don't fit it, until none are left, so rare fields and types are
  found too and a warning can say a field exists nowhere (both in Settings)
- Tree, table and JSON views; the JSON view can be selected, copied and searched
- A Schema view: every field nested under its parent, how often it appears, its types
- Explain: whether the query used an index or read the whole collection, and how many
  documents and keys it read to return how many
- Indexes: list them, create one (with field completion), drop one
- The query as driver code for Node.js, Python, Go, Java or Rust
- Query history and favorites for every collection
- Status bar with the document count and how long the query took
- Export to JSON or CSV, streamed to disk so big exports stay light

**Edit**
- Edit, clone, copy, delete and insert documents in a colored editor
- Create, rename and drop databases and collections (drops ask you to type the name)
- Helpful typing: brackets close themselves, `Obj` becomes `ObjectId("")`,
  and pasted ids, UUIDs and dates are wrapped for you

**Shell**
- A built-in shell like Studio 3T's IntelliShell: write in the editor, run it, read
  (and select, copy, search) the results below
- Completion: databases after `use`, collections after `db.`, methods after
  `db.users.` and `.find().`, and inside `find({ … })`, `aggregate([ … ])` or
  `updateOne(…)` the fields of that collection with every query operator, stage,
  expression and accumulator. In a pipeline, later stages see the fields earlier
  ones produce (after `$group`, `$project`, `$unwind`…)
- The same warnings as the query boxes, in every query in the script
- mongosh-style JavaScript on an embedded QuickJS engine:

  ```js
  show dbs
  use shop
  db.users.find({ age: { $gte: 30 } }).sort({ age: -1 }).limit(5)
  db.orders.aggregate([{ $group: { _id: "$status", n: { $sum: 1 } } }])
  db.users.find({ city: "Izmir" }).forEach(u => print(u.name))
  ```

  `find`, `findOne`, `aggregate`, `countDocuments`, `distinct`, `insertOne/Many`,
  `updateOne/Many`, `replaceOne`, `deleteOne/Many`, `findOneAndUpdate`, indexes,
  `it` for the next page, and the shell types (`ObjectId`, `ISODate`, `NumberLong`,
  `UUID`, …)

**And**
- English, Turkish, Spanish, French, Chinese (Simplified), Hindi and Arabic; adding a
  language is one file (see below). Arabic text shows, but the layout stays left to right
- A welcome on first launch to pick the language and options

## Keyboard shortcuts

| Keys | Where | Does |
| --- | --- | --- |
| ⌘K | anywhere | Quick open a collection or shell |
| ↑ ↓ Enter Esc | completion menu | Pick, insert, close |
| ↑ ↓ Enter Esc | quick open | Pick, open, close |
| ⌘-click, middle-click | collection in the sidebar | Open in a new tab |
| middle-click | tab | Close the tab |
| Enter | filter, sort, project | Run the query |
| ⌘↵ | shell | Run the script, or just the selection |
| ⌥↑ ⌥↓ | shell | Earlier and later commands |
| ⌘↵ | document editor | Save |
| ⌘F | shell results, JSON view | Search |

## Your data and privacy

- **Passwords are kept in the macOS Keychain**, under `dev.muzaffer.humon`. The rest
  of each connection (name, color, and the connection string without its password)
  is in `~/Library/Application Support/dev.muzaffer.humon/connections.json`,
  readable only by your user. Older versions kept passwords in `secrets.json`; humon
  moves them to the Keychain on start and deletes the file once all have moved.
- Because humon isn't signed yet, macOS may ask to let it use the Keychain after an
  update; choose "Always Allow". The Keychain protects passwords at rest, not on a
  machine someone else already controls.
- The same folder holds `history.json` (recent and favorite queries), `settings.json`
  and `session.json` (the open tabs, their queries and shell scripts).
- To learn a collection's fields, humon reads a sample of it and, unless turned off,
  runs a few queries for documents that don't fit the sample. Those can't use an
  index, so each is limited to 5 seconds and all of them to 20. Both are in
  Settings (⚙) if you'd rather spare a busy server.
- humon makes one network request of its own: on start it asks GitHub for the latest
  release, to show the update notice. Turn it off in Settings (⚙).

## 🤖 Built entirely by AI

Every line of code in this repo was written by an AI ([Claude](https://claude.com/claude-code)),
with a human giving directions, testing, and saying "no, not like that" a lot.
That includes this README. Treat it as an experiment: it works well for me,
but read the code before you trust it with anything important.

## Build from source

You need [Rust](https://rustup.rs) 1.88+ and macOS (Linux and Windows builds are planned).

```sh
cargo run --release
```

The first build takes a few minutes; after that it is quick. For an app with an icon
in your Dock:

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

## Tests

humon is built test-first: logic lives in plain functions with unit tests,
storage takes a directory and a secret store so tests use temp folders and an
in-memory Keychain, and live tests run against a real MongoDB.

```sh
cargo test --release

# Also against a real server (any test server; the data is created by the script):
mongosh "$URI" scripts/seed-test-db.js
HUMON_TEST_URI="$URI" cargo test --release -- --include-ignored
```

`--include-ignored` also runs `keychain_round_trip`, which writes and deletes a test
item in your real Keychain (under `dev.muzaffer.humon.test`); CI skips it.

See [CONTRIBUTING.md](CONTRIBUTING.md) for how changes are made and released, and
[CHANGELOG.md](CHANGELOG.md) for what changed in each version. Found a bug or
missing something? [Open an issue](https://github.com/yldrmzffr/humon/issues).

## To do

- [ ] Tab menu: duplicate, close others (and ⌘D)
- [ ] More shortcuts: ⌘T, ⌘W, ⌘1–9, ⌘R
- [ ] Read-only connections, so production can't be changed by accident
- [ ] Aggregation pipeline editor with stage previews
- [ ] Edit a single field without opening the whole document
- [ ] Update or delete many documents by filter (with confirmation)
- [ ] Import from JSON / CSV
- [ ] SSH tunnel and TLS certificate options
- [ ] Notarized macOS releases (no first-launch warning), then the main Homebrew repo
- [ ] Linux and Windows builds
- [ ] UI polish: fonts, sizes, spacing
- [ ] UI-level tests with gpui's test harness

## How it's built

| Path | What it does |
| --- | --- |
| `src/app/` | The window: sidebar, connections, tab bar, quick open, settings |
| `src/tab/` | A collection tab: queries and paging, tree/table/JSON views, editor, history and export panels, status bar |
| `src/console/` | The shell: QuickJS engine, mongosh-like API (`prelude.js`), history, completion, tab view |
| `src/mongo.rs` | Talking to MongoDB (runs on tokio); `CollectionRef` holds the collection operations |
| `src/query.rs` | Mongo shell syntax → BSON |
| `src/shell.rs` | Extended JSON → shell text |
| `src/highlight/` | Syntax colors: a pure lexer, and its hookup to the code editor |
| `src/complete/` | Completion and warnings for MQL: in the query boxes, in pipelines, and in the shell |
| `src/schema/` | Collection schemas: sampling, searching for what the sample missed, and how pipeline stages reshape them |
| `src/mql/` | Every operator, stage, expression and accumulator, from MongoDB's specifications |
| `src/smart.rs` | The typing helpers |
| `src/palette.rs` | Quick open's matching and ranking |
| `src/manage.rs` | Creating, renaming and dropping databases and collections |
| `src/version.rs` | Version, release notes and the update check |
| `src/store.rs` | Saved connections, with passwords in the Keychain |
| `src/history.rs`, `src/settings.rs`, `src/files.rs` | Query history, settings, and the JSON files behind them |
| `scripts/` | Release, app bundle and Homebrew cask scripts, the MQL table generator, test data |

## Thanks

- [Jérôme Tamarelle (@GromNaN)](https://github.com/GromNaN)

## License

[MIT](LICENSE). The operator definitions in `src/mql/spec.rs` are generated from
MongoDB's [MQL specifications](https://github.com/mongodb/mql-specifications)
([Apache-2.0](third_party/mql-specifications/LICENSE)) by `scripts/gen-mql.rb`.
