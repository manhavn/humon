# Changelog

All notable changes to humon. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

- Open tabs come back on the next launch, with their queries, views and
  shell scripts; their connections connect on their own
- Schema view: the collection's fields, nested, with how often each
  appears and its types
- Indexes panel: list, create (with field completion) and drop indexes
- Explain: whether a query used an index, how much it read and how long
  it took
- The query as driver code for Node.js, Python, Go, Java and Rust

- Spanish, French, Chinese (Simplified), Hindi and Arabic. Arabic text
  shows, but the layout stays left to right

## [0.1.6] - 2026-10-13

- Shell results are one block per run: the query and its run time in a
  header, with Copy result, Copy query, Run again and Delete on hover.
  Blocks fold like an accordion (a new run folds the earlier ones), failed
  runs have a red edge, and Copy all copies the whole session
- The shell editor can be made taller or shorter by dragging the divider
  under it
- Tree view: rows with long values no longer shift left (the value is cut
  with an ellipsis), and each row has a copy button
- Card buttons sit in the first row instead of covering the top rows
- The filter, sort and projection boxes no longer scroll their text away,
  and line breaks typed or pasted into them become spaces
- Settings thanks the people who help humon

## [0.1.5] - 2026-10-13

- Install with Homebrew: `brew install --cask yldrmzffr/tap/humon` (#2); each
  release updates the cask
- Passwords are stored in the macOS Keychain instead of `secrets.json` (#1).
  Existing passwords move there on start; the file is deleted once all have
  moved, and any that can't move stay usable and are retried next time

## [0.1.4] - 2026-10-13

- Connection strings with `compressors=` (as Atlas hands out) connect
  instead of failing with "compressors is an invalid option"; zlib, zstd
  and snappy are supported (#4)

- Schema discovery (#3): after sampling, humon asks the server for
  documents that don't fit what it has seen, until none are left, so rare
  fields and types are found too. Warnings then say a field exists nowhere
  rather than "not in the sample". Each query is limited to 5 seconds and
  all of them to 20; it can be turned off in Settings
- Fields found only this way show as "<1%" in suggestions

## [0.1.3] - 2026-10-13

- Completion in the shell: collections after `db.`, collection and cursor
  methods, and the collection's fields and every operator inside `find`,
  `aggregate`, `updateOne` and other calls
- Aggregation pipelines: stage names, expressions, accumulators and stage
  arguments, with later stages seeing the fields earlier ones produce
- Every operator now comes from MongoDB's MQL specifications, with its
  summary and a docs link beside the menu
- Warnings as you type, in the query boxes and the shell: unknown fields
  (with a guess), unknown operators, values of the wrong type, and operator
  arguments of the wrong shape

## [0.1.2] - 2026-10-13

- Field suggestions in the filter, sort and projection boxes: field names
  from a sample of the collection (with their types and how often they
  appear), operators that fit the field's type, subfields inside
  `$elemMatch`, and known values such as `1`/`-1` for sort
- Settings: how many documents to sample (100 to 20,000), with what more
  or fewer means
- The query boxes are colored like the shell
- A welcome on first launch, on a redesigned settings page
- The version under the sidebar links to the GitHub repo

## [0.1.1] - 2026-10-12

- Update check: on start humon asks GitHub for the latest release and shows
  a small notice under the sidebar when a newer one is out (can be turned
  off in Settings)
- Version shown in Settings and under the sidebar

## [0.1.0] - 2026-10-12

First public version.

- Saved connections with colors, a database/collection tree and collection search
- Collection tabs with filter, sort and projection in mongo shell syntax
- Tree, table and JSON views; edit, clone, copy, delete and insert documents
- Query history and favorites, export to JSON or CSV
- Create, rename and drop databases and collections
- A built-in shell on an embedded JavaScript engine, laid out like IntelliShell
- Quick open (⌘K) for any collection or shell
- English and Turkish
