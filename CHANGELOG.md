# Changelog

All notable changes to humon. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
