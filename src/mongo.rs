//! Mongo operations. Every function takes an owned `Client` so it can run on
//! tokio, outside gpui's executor.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use futures_util::TryStreamExt;
use mongodb::Client;
use mongodb::bson::{Bson, Document, doc};
use mongodb::error::ErrorKind;
use mongodb::options::ClientOptions;
use mongodb::results::CollectionType;
use serde_json::Value;

use rust_i18n::t;

use crate::query;

/// gpui runs its own executor; the driver needs tokio, so it gets a runtime of its own.
pub static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().expect("tokio runtime"));

pub fn gone() -> String {
    t!("errors.gone").into()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CollKind {
    Collection,
    View,
    Timeseries,
}

pub struct CollectionInfo {
    pub name: String,
    pub kind: CollKind,
}

pub struct DatabaseInfo {
    pub name: String,
    pub size_on_disk: u64,
}

fn err(e: mongodb::error::Error) -> String {
    e.to_string()
}

fn parse_arg(label: &str, src: &str) -> Result<Document, String> {
    query::parse_document(src).map_err(|e| format!("{label}: {e}"))
}

fn to_json(doc: Document) -> Value {
    Bson::Document(doc).into_canonical_extjson()
}

/// `_id` is held as the canonical extended JSON the UI was given, so it maps back exactly.
fn parse_id(id: &Value) -> Result<Bson, String> {
    Bson::try_from(id.clone()).map_err(|e| t!("errors.invalid_id", error = e).to_string())
}

pub async fn open(uri: String) -> Result<Client, String> {
    let mut opts = ClientOptions::parse(uri).await.map_err(err)?;
    opts.app_name = Some("humon".into());
    // Fail fast on a wrong host instead of the driver's 30s default.
    opts.server_selection_timeout = Some(Duration::from_secs(8));
    let client = Client::with_options(opts).map_err(err)?;
    // The driver connects lazily; ping so a bad URI or credentials fail here.
    client
        .database("admin")
        .run_command(doc! { "ping": 1 })
        .await
        .map_err(err)?;
    Ok(client)
}

pub async fn databases(client: Client) -> Result<Vec<DatabaseInfo>, String> {
    let mut dbs: Vec<DatabaseInfo> = client
        .list_databases()
        .await
        .map_err(err)?
        .into_iter()
        .map(|d| DatabaseInfo {
            name: d.name,
            size_on_disk: d.size_on_disk,
        })
        .collect();
    dbs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(dbs)
}

pub async fn collections(client: Client, db: String) -> Result<Vec<CollectionInfo>, String> {
    let specs: Vec<_> = client
        .database(&db)
        .list_collections()
        .await
        .map_err(err)?
        .try_collect()
        .await
        .map_err(err)?;
    let mut cols: Vec<CollectionInfo> = specs
        .into_iter()
        .map(|s| CollectionInfo {
            kind: match s.collection_type {
                CollectionType::View => CollKind::View,
                CollectionType::Timeseries => CollKind::Timeseries,
                _ => CollKind::Collection,
            },
            name: s.name,
        })
        .collect();
    cols.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(cols)
}

pub struct FindArgs {
    pub filter: String,
    pub sort: String,
    pub projection: String,
    pub skip: u64,
    pub limit: i64,
}

/// A page of documents as canonical extended JSON, and how long the server
/// round trip took (from sending the query to the last batch).
#[derive(Debug)]
pub struct Page {
    pub docs: Vec<Value>,
    pub took: Duration,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExportFormat {
    Json,
    Csv,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
        }
    }
}

/// CSV columns are taken from the first this-many documents.
const CSV_HEADER_SAMPLE: usize = 1000;

/// `_id` first, then fields in the order they first appear.
fn csv_headers(docs: &[Document]) -> Vec<String> {
    let mut headers: Vec<String> = Vec::new();
    for doc in docs {
        for k in doc.keys() {
            if !headers.contains(k) {
                headers.push(k.clone());
            }
        }
    }
    if let Some(pos) = headers.iter().position(|h| h == "_id") {
        let id = headers.remove(pos);
        headers.insert(0, id);
    }
    headers
}

/// A cell's text: plain for scalars, JSON for nested values.
fn csv_cell(value: &Bson) -> String {
    match value {
        Bson::String(s) => s.clone(),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(n) => n.to_string(),
        Bson::Boolean(b) => b.to_string(),
        Bson::Null | Bson::Undefined => String::new(),
        Bson::ObjectId(id) => id.to_hex(),
        Bson::DateTime(d) => d
            .try_to_rfc3339_string()
            .unwrap_or_else(|_| d.timestamp_millis().to_string()),
        Bson::Decimal128(d) => d.to_string(),
        other => other.clone().into_relaxed_extjson().to_string(),
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub async fn drop_database(client: Client, db: String) -> Result<(), String> {
    client.database(&db).drop().await.map_err(err)
}

#[cfg(test)]
mod csv_tests {
    use super::*;

    #[test]
    fn cells_and_escaping() {
        assert_eq!(csv_cell(&Bson::Int32(5)), "5");
        assert_eq!(csv_cell(&Bson::Null), "");
        let id = mongodb::bson::oid::ObjectId::parse_str("65a1b2c3d4e5f60718293a4b").unwrap();
        assert_eq!(csv_cell(&Bson::ObjectId(id)), "65a1b2c3d4e5f60718293a4b");
        assert_eq!(
            csv_cell(&Bson::DateTime(mongodb::bson::DateTime::from_millis(0))),
            "1970-01-01T00:00:00Z"
        );
        assert_eq!(csv_cell(&Bson::Document(doc! { "a": 1 })), r#"{"a":1}"#);
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(
            csv_headers(&[doc! { "b": 1, "_id": 2 }, doc! { "c": 3 }]),
            ["_id", "b", "c"]
        );
    }
}

/// One collection on one connection. Cheap to clone (the client is shared),
/// so each operation takes it by value and can run as a `'static` future.
#[derive(Clone)]
pub struct CollectionRef {
    pub client: Client,
    pub db: String,
    pub name: String,
}

impl CollectionRef {
    pub fn new(client: Client, db: &str, name: &str) -> Self {
        Self {
            client,
            db: db.to_string(),
            name: name.to_string(),
        }
    }

    pub fn handle(&self) -> mongodb::Collection<Document> {
        self.client.database(&self.db).collection(&self.name)
    }

    pub async fn find(self, args: FindArgs) -> Result<Page, String> {
        let filter = parse_arg("Filter", &args.filter)?;
        let sort = parse_arg("Sort", &args.sort)?;
        let projection = parse_arg("Project", &args.projection)?;
        let collection = self.handle();
        let mut find = collection
            .find(filter)
            .skip(args.skip)
            .limit(args.limit)
            .max_time(Duration::from_secs(60));
        if !sort.is_empty() {
            find = find.sort(sort);
        }
        if !projection.is_empty() {
            find = find.projection(projection);
        }
        let start = Instant::now();
        let docs: Vec<Document> = find.await.map_err(err)?.try_collect().await.map_err(err)?;
        let took = start.elapsed();
        Ok(Page {
            docs: docs.into_iter().map(to_json).collect(),
            took,
        })
    }

    /// Total for the pager. `None` means an exact count would take too long.
    pub async fn count(self, filter: String) -> Result<Option<u64>, String> {
        let filter = parse_arg("Filter", &filter)?;
        let collection = self.handle();
        if filter.is_empty() {
            // Metadata-only count; views don't support it, so fall through on error.
            if let Ok(n) = collection.estimated_document_count().await {
                return Ok(Some(n));
            }
        }
        match collection
            .count_documents(filter)
            .max_time(Duration::from_secs(5))
            .await
        {
            Ok(n) => Ok(Some(n)),
            // MaxTimeMSExpired: report "unknown" rather than failing the tab.
            Err(e) if matches!(&*e.kind, ErrorKind::Command(c) if c.code == 50) => Ok(None),
            Err(e) => Err(err(e)),
        }
    }

    /// The shape of the collection from a random sample of `size` documents.
    /// Each document is folded in and dropped, so a big sample costs time, not memory.
    pub async fn schema(self, size: u32) -> Result<crate::schema::Schema, String> {
        let mut cursor = self
            .handle()
            .aggregate([doc! { "$sample": { "size": size } }])
            .max_time(Duration::from_secs(30))
            .await
            .map_err(err)?;
        let mut builder = crate::schema::Builder::default();
        while let Some(doc) = cursor.try_next().await.map_err(err)? {
            builder.add(&doc);
        }
        Ok(builder.finish())
    }

    /// The full stored document, ignoring any projection the tab has applied.
    pub async fn get(self, id: Value) -> Result<Option<Value>, String> {
        let doc = self
            .handle()
            .find_one(doc! { "_id": parse_id(&id)? })
            .await
            .map_err(err)?;
        Ok(doc.map(to_json))
    }

    /// Replaces a document by `_id`; the edited text may not change `_id`.
    pub async fn replace(self, id: Value, src: String) -> Result<(), String> {
        let id = parse_id(&id)?;
        let edited = query::parse_document(&src)?;
        let mut doc = Document::new();
        match edited.get("_id") {
            Some(v) if *v != id => return Err(t!("errors.id_immutable").into()),
            // Keep _id first, where MongoDB stores it anyway.
            _ => doc.insert("_id", id.clone()),
        };
        doc.extend(edited.into_iter().filter(|(k, _)| k != "_id"));
        let res = self.handle().replace_one(doc! { "_id": id }, doc).await.map_err(err)?;
        if res.matched_count == 0 {
            return Err(gone());
        }
        Ok(())
    }

    pub async fn delete(self, id: Value) -> Result<(), String> {
        let res = self
            .handle()
            .delete_one(doc! { "_id": parse_id(&id)? })
            .await
            .map_err(err)?;
        if res.deleted_count == 0 {
            return Err(gone());
        }
        Ok(())
    }

    /// Inserts one document or an array of them; returns how many were written.
    pub async fn insert(self, src: String) -> Result<usize, String> {
        let docs = query::parse_documents(&src)?;
        let collection = self.handle();
        let n = docs.len();
        if n == 1 {
            collection
                .insert_one(docs.into_iter().next().unwrap())
                .await
                .map_err(err)?;
        } else {
            collection.insert_many(docs).await.map_err(err)?;
        }
        Ok(n)
    }

    /// Writes the query's results to `path`, streaming from the cursor so memory
    /// stays flat however many documents match. `limit` 0 exports everything.
    /// JSON is an array of relaxed extended JSON; CSV has one column per
    /// top-level field, with nested values written as JSON.
    pub async fn export(self, args: FindArgs, format: ExportFormat, path: std::path::PathBuf) -> Result<u64, String> {
        use std::io::Write;

        let filter = parse_arg("Filter", &args.filter)?;
        let sort = parse_arg("Sort", &args.sort)?;
        let projection = parse_arg("Project", &args.projection)?;
        let collection = self.handle();
        let mut find = collection.find(filter).skip(args.skip);
        if args.limit > 0 {
            find = find.limit(args.limit);
        }
        if !sort.is_empty() {
            find = find.sort(sort);
        }
        if !projection.is_empty() {
            find = find.projection(projection);
        }
        let mut cursor = find.await.map_err(err)?;
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let mut out = std::io::BufWriter::new(file);
        let io = |e: std::io::Error| e.to_string();
        let mut written = 0u64;

        match format {
            ExportFormat::Json => {
                out.write_all(b"[").map_err(io)?;
                while let Some(doc) = cursor.try_next().await.map_err(err)? {
                    out.write_all(if written == 0 { b"\n" } else { b",\n" }).map_err(io)?;
                    serde_json::to_writer(&mut out, &Bson::Document(doc).into_relaxed_extjson())
                        .map_err(|e| e.to_string())?;
                    written += 1;
                }
                out.write_all(if written == 0 { b"]\n" } else { b"\n]\n" })
                    .map_err(io)?;
            }
            ExportFormat::Csv => {
                let mut sample = Vec::new();
                while sample.len() < CSV_HEADER_SAMPLE {
                    match cursor.try_next().await.map_err(err)? {
                        Some(doc) => sample.push(doc),
                        None => break,
                    }
                }
                let headers = csv_headers(&sample);
                writeln!(
                    out,
                    "{}",
                    headers.iter().map(|h| csv_escape(h)).collect::<Vec<_>>().join(",")
                )
                .map_err(io)?;
                let write_row = |doc: &Document, out: &mut std::io::BufWriter<std::fs::File>| -> Result<(), String> {
                    let row: Vec<String> = headers
                        .iter()
                        .map(|h| doc.get(h).map(csv_cell).map(|c| csv_escape(&c)).unwrap_or_default())
                        .collect();
                    writeln!(out, "{}", row.join(",")).map_err(io)
                };
                for doc in &sample {
                    write_row(doc, &mut out)?;
                    written += 1;
                }
                while let Some(doc) = cursor.try_next().await.map_err(err)? {
                    write_row(&doc, &mut out)?;
                    written += 1;
                }
            }
        }
        out.flush().map_err(io)?;
        Ok(written)
    }

    /// Creates the collection; an existing name is an error (MongoDB 7+
    /// would silently accept it).
    pub async fn create(self) -> Result<(), String> {
        let database = self.client.database(&self.db);
        if database
            .list_collection_names()
            .await
            .map_err(err)?
            .contains(&self.name)
        {
            return Err(t!("errors.collection_exists", name = self.name).into());
        }
        database.create_collection(&self.name).await.map_err(err)
    }

    pub async fn drop(self) -> Result<(), String> {
        self.handle().drop().await.map_err(err)
    }

    pub async fn rename(self, to: String) -> Result<(), String> {
        let (db, from) = (&self.db, &self.name);
        self.client
            .database("admin")
            .run_command(doc! { "renameCollection": format!("{db}.{from}"), "to": format!("{db}.{to}") })
            .await
            .map(|_| ())
            .map_err(err)
    }
}

/// Live tests: `HUMON_TEST_URI=mongodb://... cargo test -- --ignored`, with
/// `scripts/seed-test-db.js` loaded. Each writing test uses its own scratch
/// collection or database and drops it, so the tests can run in parallel.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn client() -> Client {
        open(std::env::var("HUMON_TEST_URI").expect("HUMON_TEST_URI"))
            .await
            .unwrap()
    }

    async fn users() -> CollectionRef {
        CollectionRef::new(client().await, "shop", "users")
    }

    /// A fresh, empty collection in `shop` for one test.
    async fn scratch(name: &str) -> CollectionRef {
        let coll = CollectionRef::new(client().await, "shop", name);
        coll.clone().drop().await.ok();
        coll
    }

    fn args(filter: &str, sort: &str, projection: &str, skip: u64, limit: i64) -> FindArgs {
        FindArgs {
            filter: filter.into(),
            sort: sort.into(),
            projection: projection.into(),
            skip,
            limit,
        }
    }

    #[tokio::test]
    #[ignore = "needs a live MongoDB"]
    async fn live_browse() {
        let c = client().await;
        let dbs: Vec<String> = databases(c.clone())
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert!(dbs.contains(&"shop".to_string()), "{dbs:?}");
        let cols = collections(c, "shop".into()).await.unwrap();
        assert!(cols.iter().any(|x| x.name == "adults" && x.kind == CollKind::View));
        assert!(cols.iter().any(|x| x.name == "users" && x.kind == CollKind::Collection));
    }

    #[tokio::test]
    #[ignore = "needs a live MongoDB"]
    async fn live_schema() {
        let schema = users().await.schema(1000).await.unwrap();
        assert_eq!(schema.sampled, 137, "asking for more than exists samples them all");
        let city = schema.field("address.city").unwrap();
        assert_eq!((city.count, city.types.as_slice()), (137, &[("string", 137)][..]));
        assert_eq!(schema.field("big").unwrap().types, [("long", 137)]);
        assert_eq!(schema.field("tags").unwrap().items, [("string", 274)]);
        assert_eq!(users().await.schema(10).await.unwrap().sampled, 10);
        let view = CollectionRef::new(client().await, "shop", "adults");
        assert!(
            view.schema(50).await.unwrap().field("age").is_some(),
            "views can be sampled"
        );
    }

    #[tokio::test]
    #[ignore = "needs a live MongoDB"]
    async fn live_count() {
        // Ages cycle 18..=67 over 137 users; city is Izmir when i % 3 == 0.
        assert_eq!(users().await.count("".into()).await.unwrap(), Some(137));
        assert_eq!(
            users().await.count("{ 'address.city': 'Izmir' }".into()).await.unwrap(),
            Some(46)
        );
        let view = CollectionRef::new(client().await, "shop", "adults");
        assert!(
            view.count("".into()).await.unwrap().unwrap() > 0,
            "views fall back to an exact count"
        );
        assert!(
            users()
                .await
                .count("{ age: ".into())
                .await
                .unwrap_err()
                .starts_with("Filter:")
        );
    }

    #[tokio::test]
    #[ignore = "needs a live MongoDB"]
    async fn live_find() {
        let page = users()
            .await
            .find(args(
                "{ age: { $gte: 60 } }",
                "{ age: -1, name: 1 }",
                "{ _id: 0, name: 1, age: 1, big: 1 }",
                0,
                5,
            ))
            .await
            .unwrap()
            .docs;
        assert_eq!(page.len(), 5);
        let first = page[0].as_object().unwrap();
        // Projection keeps the stored field order; types survive as canonical EJSON.
        assert_eq!(first.keys().collect::<Vec<_>>(), ["name", "age", "big"]);
        assert_eq!(first["age"], json!({ "$numberInt": "67" }));
        assert_eq!(first["big"], json!({ "$numberLong": "9007199254740993" }));
        let len = |a| async move { users().await.find(a).await.unwrap().docs.len() };
        assert_eq!(len(args("", "{ name: 1 }", "", 135, 20)).await, 2, "skip near the end");
        assert_eq!(
            len(args("{ createdAt: { $lt: ISODate('2024-01-03') } }", "", "", 0, 50)).await,
            2
        );
        assert_eq!(len(args("{ name: /^user1\\d$/ }", "", "", 0, 50)).await, 10);
        let bad = users().await.find(args("{ age: ", "", "", 0, 5)).await.unwrap_err();
        assert!(bad.starts_with("Filter:"), "{bad}");
    }

    #[tokio::test]
    #[ignore = "needs a live MongoDB"]
    async fn live_writes() {
        let coll = scratch("humon_test_writes").await;
        assert_eq!(coll.clone().insert("[{ n: 1 }, { n: 2 }]".into()).await.unwrap(), 2);
        assert_eq!(
            coll.clone()
                .insert("{ _id: 'k', n: 3, at: ISODate('2024-05-01') }".into())
                .await
                .unwrap(),
            1
        );
        let key = json!("k");
        coll.clone()
            .replace(key.clone(), "{ n: NumberLong(4), extra: true }".into())
            .await
            .unwrap();
        let stored = coll.clone().get(key.clone()).await.unwrap().unwrap();
        assert_eq!(
            stored,
            json!({ "_id": "k", "n": { "$numberLong": "4" }, "extra": true }),
            "replace drops fields left out"
        );
        let changed_id = coll
            .clone()
            .replace(key.clone(), "{ _id: 'other' }".into())
            .await
            .unwrap_err();
        assert_eq!(changed_id, t!("errors.id_immutable"));
        coll.clone().delete(key.clone()).await.unwrap();
        assert_eq!(coll.clone().delete(key.clone()).await.unwrap_err(), gone());
        assert_eq!(coll.clone().replace(key, "{}".into()).await.unwrap_err(), gone());
        coll.drop().await.unwrap();
    }

    /// Opening a document in the editor and saving it unchanged must store
    /// exactly the same BSON, types included.
    #[tokio::test]
    #[ignore = "needs a live MongoDB"]
    async fn live_unchanged_edit_round_trips_every_type() {
        use mongodb::bson::{Binary, DateTime, Decimal128, Regex, Timestamp, oid::ObjectId, spec::BinarySubtype};
        let coll = scratch("humon_test_round_trip").await;
        let exotic = doc! {
            "_id": ObjectId::new(),
            "i32": 7, "i64": 9_007_199_254_740_993_i64, "whole": 1.0, "frac": 0.1, "neg": -2.5,
            "dec": "1.10".parse::<Decimal128>().unwrap(),
            "at": DateTime::from_millis(1_704_153_845_006),
            "old": DateTime::from_millis(-86_400_000),
            "far": DateTime::from_millis(253_402_300_800_000),
            "nan": f64::NAN, "inf": f64::INFINITY, "ninf": f64::NEG_INFINITY,
            "uuid": Binary { subtype: BinarySubtype::Uuid, bytes: vec![0x12; 16] },
            "bin": Binary { subtype: BinarySubtype::Generic, bytes: vec![1, 2, 3] },
            "re": Regex { pattern: "^a\\/b".into(), options: "i".into() },
            "ts": Timestamp { time: 10, increment: 2 },
            "none": Bson::Null, "yes": true,
            "text": "quote \" backslash \\ newline \n tab \t emoji 🍃 Çağrı",
            "nested": { "list": [1, "two", { "three": 3_i64 }], "empty": {}, "arr": [] },
            "min": Bson::MinKey, "max": Bson::MaxKey,
        };
        let id = Bson::from(exotic.get_object_id("_id").unwrap()).into_canonical_extjson();
        coll.handle().insert_one(exotic).await.unwrap();
        let before = coll.clone().get(id.clone()).await.unwrap().unwrap();
        let editor_text = crate::shell::to_shell(&before, "");
        coll.clone().replace(id.clone(), editor_text.clone()).await.unwrap();
        let after = coll.clone().get(id).await.unwrap().unwrap();
        assert_eq!(after, before, "editor text was:\n{editor_text}");
        coll.drop().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a live MongoDB"]
    async fn live_export() {
        let dir = crate::files::test_dir("export");
        let json_path = dir.join("users.json");
        let n = users()
            .await
            .export(
                args("{ age: { $gte: 60 } }", "{ name: 1 }", "", 0, 0),
                ExportFormat::Json,
                json_path.clone(),
            )
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(parsed.len() as u64, n);
        assert_eq!(
            Some(n),
            users().await.count("{ age: { $gte: 60 } }".into()).await.unwrap()
        );
        assert!(
            parsed[0]["_id"]["$oid"].is_string(),
            "relaxed EJSON keeps ObjectId typed"
        );
        let csv_path = dir.join("users.csv");
        let n = users()
            .await
            .export(
                args("", "{ name: 1 }", "{ name: 1, age: 1 }", 0, 3),
                ExportFormat::Csv,
                csv_path.clone(),
            )
            .await
            .unwrap();
        let csv = std::fs::read_to_string(&csv_path).unwrap();
        assert_eq!(n, 3);
        assert_eq!(csv.lines().next(), Some("_id,name,age"));
        assert_eq!(csv.lines().count(), 4);
    }

    #[tokio::test]
    #[ignore = "needs a live MongoDB"]
    async fn live_manage_databases_and_collections() {
        let c = client().await;
        let tmp = "humon_test_manage".to_string();
        drop_database(c.clone(), tmp.clone()).await.unwrap(); // leftovers from an aborted run
        let a = CollectionRef::new(c.clone(), &tmp, "a");
        a.clone().create().await.unwrap();
        assert!(a.clone().create().await.is_err(), "an existing name is an error");
        a.rename("b".into()).await.unwrap();
        let names: Vec<String> = collections(c.clone(), tmp.clone())
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.name)
            .collect();
        assert_eq!(names, ["b"]);
        CollectionRef::new(c.clone(), &tmp, "b").drop().await.unwrap();
        CollectionRef::new(c.clone(), &tmp, "c").create().await.unwrap();
        drop_database(c.clone(), tmp.clone()).await.unwrap();
        let dbs: Vec<String> = databases(c).await.unwrap().into_iter().map(|d| d.name).collect();
        assert!(!dbs.contains(&tmp));
    }
}
