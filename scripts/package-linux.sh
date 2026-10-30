#!/bin/sh
set -eu

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
echo "Packaging humon v$VERSION for Linux (amd64 & aarch64)..."

rm -rf dist/*
mkdir -p dist

# amd64
tar -czf "dist/humon-v${VERSION}-linux-amd64.tar.gz" -C target/release humon

# arm64
tar -czf "dist/humon-v${VERSION}-linux-arm64.tar.gz" -C target/aarch64-unknown-linux-gnu/release humon

# Checksums
sha256sum dist/humon-*.tar.gz | sed 's|dist/||' > dist/SHA256SUMS.txt

# Release notes
target/release/humon --release-notes "$VERSION" > dist/notes.md

echo "Packaging complete in dist/:"
ls -lh dist/
