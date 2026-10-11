//! Mongo operations, ported from src-tauri/src/mongo.rs without the command
//! wrappers. Every function takes an owned `Client` so it can run on tokio.

use std::sync::LazyLock;
use std::time::Duration;

use futures_util::TryStreamExt;
use mongodb::Client;
use mongodb::bson::{Bson, Document, doc};
use mongodb::error::ErrorKind;
use mongodb::options::ClientOptions;
use mongodb::results::CollectionType;
use serde_json::Value;

use crate::query;

/// gpui runs its own executor; the driver needs tokio, so it gets a runtime of its own.
pub static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().expect("tokio runtime"));

pub const GONE: &str = "doküman bulunamadı; başka biri silmiş olabilir";

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
    Bson::try_from(id.clone()).map_err(|e| format!("geçersiz _id: {e}"))
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

/// One page of documents as canonical extended JSON.
pub async fn find(client: Client, db: String, coll: String, args: FindArgs) -> Result<Vec<Value>, String> {
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
    let docs: Vec<Document> = find.await.map_err(err)?.try_collect().await.map_err(err)?;
    Ok(docs.into_iter().map(to_json).collect())
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
        Some(v) if *v != id => return Err("_id değiştirilemez".into()),
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
        return Err(GONE.into());
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
        return Err(GONE.into());
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
