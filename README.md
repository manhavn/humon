# humon

Hafif bir MongoDB istemcisi; Compass'a alternatif. Arayüz [gpui](https://www.gpui.rs)
ile (tarayıcı motoru yok), veritabanı tarafı resmi Rust `mongodb` sürücüsüyle.

Aynı veriyle ölçüm (100 doküman açık): humon ~33-41 MB, Compass ~290 MB.

## Çalıştırma

```sh
cargo run --release
```

İlk derleme birkaç dakika sürer; sonrakiler saniyeler.

## Özellikler

- Kayıtlı bağlantılar (`~/Library/Application Support/dev.muzaffer.humon/`)
- Bağlantı → veritabanı → collection ağacı, collection başına sekme
  (⌘ + tık veya orta tık yeni sekme)
- Filter / Sort / Project shell sözdizimiyle: `{ _id: ObjectId("…"), at: { $gt: ISODate("2024-01-01") } }`
- Renkli doküman ağacı; düzenle, klonla, kopyala, sil, ekle
- Yazarken: parantez/tırnak kapatma, `Obj` → `ObjectId("")` tamamlama,
  yapıştırılan id/UUID/tarihi sarma

## Kod

| Dosya | İçerik |
| --- | --- |
| `src/app.rs` | Ana pencere, kenar çubuğu, sekme çubuğu |
| `src/tab.rs` | Collection sekmesi: sorgu, sayfalama, doküman ağacı, editörler |
| `src/mongo.rs` | Mongo işlemleri (tokio üzerinde) |
| `src/query.rs` | Shell sözdizimi → BSON |
| `src/shell.rs` | Extended JSON → shell metni |
| `src/highlight.rs` | Renklendirme için sözdizimi ayrıştırıcı |
| `src/smart.rs` | Yazma yardımcıları |
| `src/store.rs` | Kayıtlı bağlantılar |

## Test

```sh
cargo test --release
# Canlı sunucuya karşı (shop seed verisiyle):
HUMON_TEST_URI=mongodb://127.0.0.1:27018 cargo test --release -- --include-ignored
```

## Bilinen eksikler

- Şifreler `secrets.json` içinde açık metin (dosya izni 0600); Keychain'e taşınacak.
- `.app` paketi yok; şimdilik `target/release/humon`.
