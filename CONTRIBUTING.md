# Contributing

Thanks for helping! A few things keep humon healthy:

## Test first

Every change comes with a test, written before the code. Keep logic in plain
functions so it can be tested without a window; UI code should mostly wire
those functions together.

```sh
cargo test --release
```

The live tests talk to a real MongoDB. Start one on port 27018, load the test
data, and include them:

```sh
mongosh mongodb://127.0.0.1:27018 scripts/seed-test-db.js
HUMON_TEST_URI=mongodb://127.0.0.1:27018 cargo test --release -- --include-ignored
```

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

Adding a language is one file in `locales/`; see the README. `cargo test`
tells you if a key is missing.

## Keep it light

humon exists because MongoDB Compass uses too much memory. Before adding a
dependency, check what it costs in binary size and RAM.
