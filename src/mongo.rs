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
        .map(|d| DatabaseInfo { name: d.name, size_on_disk: d.size_on_disk })
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

pub async fn find(client: Client, db: String, coll: String, args: FindArgs) -> Result<Page, String> {
    let filter = parse_arg("Filter", &args.filter)?;
    let sort = parse_arg("Sort", &args.sort)?;
    let projection = parse_arg("Project", &args.projection)?;
    let collection = client.database(&db).collection::<Document>(&coll);
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
    Ok(Page { docs: docs.into_iter().map(to_json).collect(), took })
}

/// Total for the pager. `None` means an exact count would take too long.
pub async fn count(client: Client, db: String, coll: String, filter: String) -> Result<Option<u64>, String> {
    let filter = parse_arg("Filter", &filter)?;
    let collection = client.database(&db).collection::<Document>(&coll);
    if filter.is_empty() {
        // Metadata-only count; views don't support it, so fall through on error.
        if let Ok(n) = collection.estimated_document_count().await {
            return Ok(Some(n));
        }
    }
    match collection.count_documents(filter).max_time(Duration::from_secs(5)).await {
        Ok(n) => Ok(Some(n)),
        // MaxTimeMSExpired: report "unknown" rather than failing the tab.
        Err(e) if matches!(&*e.kind, ErrorKind::Command(c) if c.code == 50) => Ok(None),
        Err(e) => Err(err(e)),
    }
}

/// The full stored document, ignoring any projection the tab has applied.
pub async fn get(client: Client, db: String, coll: String, id: Value) -> Result<Option<Value>, String> {
    let doc = client
        .database(&db)
        .collection::<Document>(&coll)
        .find_one(doc! { "_id": parse_id(&id)? })
        .await
        .map_err(err)?;
    Ok(doc.map(to_json))
}

/// Replaces a document by `_id`; the edited text may not change `_id`.
pub async fn replace(client: Client, db: String, coll: String, id: Value, src: String) -> Result<(), String> {
    let id = parse_id(&id)?;
    let edited = query::parse_document(&src)?;
    let mut doc = Document::new();
    match edited.get("_id") {
        Some(v) if *v != id => return Err(t!("errors.id_immutable").into()),
        // Keep _id first, where MongoDB stores it anyway.
        _ => doc.insert("_id", id.clone()),
    };
    doc.extend(edited.into_iter().filter(|(k, _)| k != "_id"));
    let res = client
        .database(&db)
        .collection::<Document>(&coll)
        .replace_one(doc! { "_id": id }, doc)
        .await
        .map_err(err)?;
    if res.matched_count == 0 {
        return Err(gone());
    }
    Ok(())
}

pub async fn delete(client: Client, db: String, coll: String, id: Value) -> Result<(), String> {
    let res = client
        .database(&db)
        .collection::<Document>(&coll)
        .delete_one(doc! { "_id": parse_id(&id)? })
        .await
        .map_err(err)?;
    if res.deleted_count == 0 {
        return Err(gone());
    }
    Ok(())
}

/// Inserts one document or an array of them; returns how many were written.
pub async fn insert(client: Client, db: String, coll: String, src: String) -> Result<usize, String> {
    let docs = query::parse_documents(&src)?;
    let collection = client.database(&db).collection::<Document>(&coll);
    let n = docs.len();
    if n == 1 {
        collection.insert_one(docs.into_iter().next().unwrap()).await.map_err(err)?;
    } else {
        collection.insert_many(docs).await.map_err(err)?;
    }
    Ok(n)
}

/// Runs against a live server: `HUMON_TEST_URI=mongodb://... cargo test -- --ignored`.
/// Expects the `shop` seed data (137 users, an `adults` view); writes go to a
/// scratch collection that is dropped at the end.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(filter: &str, sort: &str, projection: &str, skip: u64, limit: i64) -> FindArgs {
        FindArgs { filter: filter.into(), sort: sort.into(), projection: projection.into(), skip, limit }
    }

    #[tokio::test]
    #[ignore]
    async fn live_server() {
        let uri = std::env::var("HUMON_TEST_URI").expect("HUMON_TEST_URI");
        let c = open(uri).await.unwrap();
        let (db, users) = ("shop".to_string(), "users".to_string());

        let dbs: Vec<String> = databases(c.clone()).await.unwrap().into_iter().map(|d| d.name).collect();
        assert!(dbs.contains(&db), "{dbs:?}");
        let cols = collections(c.clone(), db.clone()).await.unwrap();
        assert!(cols.iter().any(|x| x.name == "adults" && x.kind == CollKind::View));

        // Ages cycle 18..=67; city is Izmir when i % 3 == 0.
        assert_eq!(count(c.clone(), db.clone(), users.clone(), "".into()).await.unwrap(), Some(137));
        assert_eq!(count(c.clone(), db.clone(), users.clone(), "{ 'address.city': 'Izmir' }".into()).await.unwrap(), Some(46));
        assert!(count(c.clone(), db.clone(), "adults".into(), "".into()).await.unwrap().unwrap() > 0);

        let page = find(c.clone(), db.clone(), users.clone(), args("{ age: { $gte: 60 } }", "{ age: -1, name: 1 }", "{ _id: 0, name: 1, age: 1, big: 1 }", 0, 5))
            .await
            .unwrap()
            .docs;
        assert_eq!(page.len(), 5);
        let first = page[0].as_object().unwrap();
        assert_eq!(first.keys().collect::<Vec<_>>(), ["name", "age", "big"]);
        assert_eq!(first["age"], json!({ "$numberInt": "67" }));
        assert_eq!(first["big"], json!({ "$numberLong": "9007199254740993" }));
        assert_eq!(find(c.clone(), db.clone(), users.clone(), args("", "{ name: 1 }", "", 135, 20)).await.unwrap().docs.len(), 2);
        assert_eq!(find(c.clone(), db.clone(), users.clone(), args("{ createdAt: { $lt: ISODate('2024-01-03') } }", "", "", 0, 50)).await.unwrap().docs.len(), 2);
        assert_eq!(find(c.clone(), db.clone(), users.clone(), args("{ name: /^user1\\d$/ }", "", "", 0, 50)).await.unwrap().docs.len(), 10);
        let bad = find(c.clone(), db.clone(), users.clone(), args("{ age: ", "", "", 0, 5)).await.unwrap_err();
        assert!(bad.starts_with("Filter:"), "{bad}");

        let scratch = "humon_write_test".to_string();
        assert_eq!(insert(c.clone(), db.clone(), scratch.clone(), "[{ n: 1 }, { n: 2 }]".into()).await.unwrap(), 2);
        assert_eq!(insert(c.clone(), db.clone(), scratch.clone(), "{ _id: 'k', n: 3, at: ISODate('2024-05-01') }".into()).await.unwrap(), 1);
        let key = json!("k");
        replace(c.clone(), db.clone(), scratch.clone(), key.clone(), "{ n: NumberLong(4), extra: true }".into()).await.unwrap();
        let stored = get(c.clone(), db.clone(), scratch.clone(), key.clone()).await.unwrap().unwrap();
        assert_eq!(stored, json!({ "_id": "k", "n": { "$numberLong": "4" }, "extra": true }));
        let changed_id = replace(c.clone(), db.clone(), scratch.clone(), key.clone(), "{ _id: 'other' }".into()).await.unwrap_err();
        assert_eq!(changed_id, t!("errors.id_immutable"));
        delete(c.clone(), db.clone(), scratch.clone(), key.clone()).await.unwrap();
        assert_eq!(delete(c.clone(), db.clone(), scratch.clone(), key.clone()).await.unwrap_err(), gone());
        assert_eq!(replace(c.clone(), db.clone(), scratch.clone(), key, "{}".into()).await.unwrap_err(), gone());
        c.database(&db).collection::<Document>(&scratch).drop().await.unwrap();
    }
}
