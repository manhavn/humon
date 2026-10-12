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

## Translations

Adding a language is one file in `locales/`; see the README. `cargo test`
tells you if a key is missing.

## Keep it light

humon exists because MongoDB Compass uses too much memory. Before adding a
dependency, check what it costs in binary size and RAM.
