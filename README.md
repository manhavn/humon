# humon

Hafif bir MongoDB Compass alternatifi. Tauri 2 (Rust) + Svelte 5.

- `pnpm tauri dev`: geliştirme
- `pnpm tauri build --no-bundle`: release binary (`src-tauri/target/release/humon`)

Mimari kuralı: BSON Rust tarafında kalır, UI'a sadece görünen sayfa gider.
