#!/bin/sh
# Prepares a release: sets the version, checks the changelog, tests, commits
# and tags. Pushing the tag starts the release workflow.
#
#   scripts/release.sh 0.2.0
set -eu
cd "$(dirname "$0")/.."

version=${1:-}
echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || { echo "usage: scripts/release.sh X.Y.Z" >&2; exit 1; }
git diff --quiet && git diff --cached --quiet || { echo "commit or stash your changes first" >&2; exit 1; }
git rev-parse -q --verify "refs/tags/v$version" >/dev/null && { echo "tag v$version already exists" >&2; exit 1; }
grep -q "^## \[$version\]" CHANGELOG.md || { echo "add a '## [$version] - $(date +%F)' section to CHANGELOG.md first" >&2; exit 1; }

# Only the package's own `version = ` line starts a line; dependencies use inline tables.
sed -i.bak "s/^version = \".*\"/version = \"$version\"/" Cargo.toml && rm Cargo.toml.bak
cargo test --release
git commit -qam "Release v$version"
git tag "v$version"
echo "Ready. Publish with: git push origin main v$version"
