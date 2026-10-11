use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::TryStreamExt;
use mongodb::Client;
use mongodb::bson::{Bson, Document, doc};
use mongodb::error::ErrorKind;
use mongodb::options::ClientOptions;
use mongodb::results::CollectionType;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio::sync::RwLock;

use crate::{query, store};

/// Open connections, keyed by an id handed to the UI.
#[derive(Default)]
pub struct Connections {
    next_id: AtomicU64,
    clients: RwLock<HashMap<String, Client>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseInfo {
    name: String,
    size_on_disk: u64,
}

#[derive(Serialize)]
pub struct CollectionInfo {
    name: String,
    kind: &'static str,
}

/// Query bar input, still in shell syntax.
#[derive(Deserialize, Default)]
pub struct FindArgs {
    filter: String,
    sort: String,
    projection: String,
    skip: u64,
    limit: i64,
}

fn err(e: mongodb::error::Error) -> String {
    e.to_string()
}

fn parse_arg(label: &str, src: &str) -> Result<Document, String> {
    query::parse_document(src).map_err(|e| format!("{label}: {e}"))
}

/// `_id` arrives as the canonical extended JSON the UI was given, so it maps back exactly.
fn parse_id(id: serde_json::Value) -> Result<Bson, String> {
    Bson::try_from(id).map_err(|e| format!("geçersiz _id: {e}"))
}

fn to_json(doc: Document) -> serde_json::Value {
    Bson::Document(doc).into_canonical_extjson()
}

const GONE: &str = "doküman bulunamadı; başka biri silmiş olabilir";

impl Connections {
    async fn client(&self, id: &str) -> Result<Client, String> {
        // Client is a cheap Arc handle, so cloning it out keeps the lock short.
        self.clients
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| format!("bağlantı bulunamadı: {id}"))
    }

    async fn open(&self, uri: &str) -> Result<String, String> {
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

        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        self.clients.write().await.insert(id.clone(), client);
        Ok(id)
    }

    async fn close(&self, id: &str) {
        if let Some(client) = self.clients.write().await.remove(id) {
            client.shutdown().await;
        }
    }

    async fn databases(&self, id: &str) -> Result<Vec<DatabaseInfo>, String> {
        let client = self.client(id).await?;
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

    async fn collections(&self, id: &str, db: &str) -> Result<Vec<CollectionInfo>, String> {
        let client = self.client(id).await?;
        let specs: Vec<_> = client
            .database(db)
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
                    CollectionType::View => "view",
                    CollectionType::Timeseries => "timeseries",
                    _ => "collection",
                },
                name: s.name,
            })
            .collect();
        cols.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(cols)
    }

    async fn find(
        &self,
        id: &str,
        db: &str,
        coll: &str,
        args: FindArgs,
    ) -> Result<Vec<serde_json::Value>, String> {
        let filter = parse_arg("Filter", &args.filter)?;
        let sort = parse_arg("Sort", &args.sort)?;
        let projection = parse_arg("Project", &args.projection)?;
        let client = self.client(id).await?;
        let collection = client.database(db).collection::<Document>(coll);

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

    /// The full stored document, ignoring any projection the tab has applied.
    async fn get(
        &self,
        id: &str,
        db: &str,
        coll: &str,
        doc_id: Bson,
    ) -> Result<Option<serde_json::Value>, String> {
        let client = self.client(id).await?;
        let doc = client
            .database(db)
            .collection::<Document>(coll)
            .find_one(doc! { "_id": doc_id })
            .await
            .map_err(err)?;
        Ok(doc.map(to_json))
    }

    /// Replaces a document by `_id`. Returns the stored result.
    async fn replace(
        &self,
        id: &str,
        db: &str,
        coll: &str,
        doc_id: Bson,
        src: &str,
    ) -> Result<serde_json::Value, String> {
        let edited = query::parse_document(src)?;
        let mut doc = Document::new();
        match edited.get("_id") {
            Some(v) if *v != doc_id => return Err("_id değiştirilemez".into()),
            // Keep _id first, where MongoDB stores it anyway.
            _ => doc.insert("_id", doc_id.clone()),
        };
        doc.extend(edited.into_iter().filter(|(k, _)| k != "_id"));

        let client = self.client(id).await?;
        let collection = client.database(db).collection::<Document>(coll);
        let res = collection
            .replace_one(doc! { "_id": doc_id }, doc.clone())
            .await
            .map_err(err)?;
        if res.matched_count == 0 {
            return Err(GONE.into());
        }
        Ok(to_json(doc))
    }

    async fn delete(&self, id: &str, db: &str, coll: &str, doc_id: Bson) -> Result<(), String> {
        let client = self.client(id).await?;
        let res = client
            .database(db)
            .collection::<Document>(coll)
            .delete_one(doc! { "_id": doc_id })
            .await
            .map_err(err)?;
        if res.deleted_count == 0 {
            return Err(GONE.into());
        }
        Ok(())
    }

    /// Inserts one document or an array of them; returns how many were written.
    async fn insert(&self, id: &str, db: &str, coll: &str, src: &str) -> Result<usize, String> {
        let docs = query::parse_documents(src)?;
        let client = self.client(id).await?;
        let collection = client.database(db).collection::<Document>(coll);
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

    async fn count(
        &self,
        id: &str,
        db: &str,
        coll: &str,
        filter: &str,
    ) -> Result<Option<u64>, String> {
        let filter = parse_arg("Filter", filter)?;
        let client = self.client(id).await?;
        let collection = client.database(db).collection::<Document>(coll);

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
}

type State<'a> = tauri::State<'a, Connections>;

#[tauri::command]
pub async fn connect(uri: String, state: State<'_>) -> Result<String, String> {
    state.open(uri.trim()).await
}

/// Connects using a saved connection, so the password never passes through the UI.
#[tauri::command]
pub async fn connect_saved(
    app: AppHandle,
    saved_id: String,
    state: State<'_>,
) -> Result<String, String> {
    let uri = store::resolve_uri(&app, &saved_id)?;
    state.open(&uri).await
}

#[tauri::command]
pub async fn disconnect(id: String, state: State<'_>) -> Result<(), String> {
    state.close(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn list_databases(id: String, state: State<'_>) -> Result<Vec<DatabaseInfo>, String> {
    state.databases(&id).await
}

#[tauri::command]
pub async fn list_collections(
    id: String,
    db: String,
    state: State<'_>,
) -> Result<Vec<CollectionInfo>, String> {
    state.collections(&id, &db).await
}

/// One page of documents as canonical extended JSON, so the UI can tell
/// ObjectId, dates, int/long/double and decimals apart.
#[tauri::command]
pub async fn find_documents(
    id: String,
    db: String,
    coll: String,
    args: FindArgs,
    state: State<'_>,
) -> Result<Vec<serde_json::Value>, String> {
    state.find(&id, &db, &coll, args).await
}

/// Total for the pager. `None` means an exact count would take too long.
#[tauri::command]
pub async fn count_documents(
    id: String,
    db: String,
    coll: String,
    filter: String,
    state: State<'_>,
) -> Result<Option<u64>, String> {
    state.count(&id, &db, &coll, &filter).await
}

#[tauri::command]
pub async fn get_document(
    id: String,
    db: String,
    coll: String,
    doc_id: serde_json::Value,
    state: State<'_>,
) -> Result<Option<serde_json::Value>, String> {
    state.get(&id, &db, &coll, parse_id(doc_id)?).await
}

#[tauri::command]
pub async fn replace_document(
    id: String,
    db: String,
    coll: String,
    doc_id: serde_json::Value,
    doc: String,
    state: State<'_>,
) -> Result<serde_json::Value, String> {
    state
        .replace(&id, &db, &coll, parse_id(doc_id)?, &doc)
        .await
}

#[tauri::command]
pub async fn delete_document(
    id: String,
    db: String,
    coll: String,
    doc_id: serde_json::Value,
    state: State<'_>,
) -> Result<(), String> {
    state.delete(&id, &db, &coll, parse_id(doc_id)?).await
}

#[tauri::command]
pub async fn insert_documents(
    id: String,
    db: String,
    coll: String,
    docs: String,
    state: State<'_>,
) -> Result<usize, String> {
    state.insert(&id, &db, &coll, &docs).await
}

/// Runs against a live server: `HUMON_TEST_URI=mongodb://... cargo test -- --ignored`.
/// Expects the seed data described in the test body.
#[cfg(test)]
mod tests {
    use super::*;

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
    #[ignore]
    async fn live_server() {
        let uri = std::env::var("HUMON_TEST_URI").expect("HUMON_TEST_URI");
        let conns = Connections::default();
        let id = conns.open(&uri).await.unwrap();

        let dbs: Vec<String> = conns
            .databases(&id)
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert!(dbs.contains(&"shop".to_string()), "{dbs:?}");

        let cols = conns.collections(&id, "shop").await.unwrap();
        let kinds: Vec<_> = cols.iter().map(|c| (c.name.as_str(), c.kind)).collect();
        assert!(kinds.contains(&("adults", "view")), "{kinds:?}");
        assert!(kinds.contains(&("users", "collection")));

        // 137 users, ages 18..=67 cycling; city is Izmir when i % 3 == 0.
        assert_eq!(
            conns.count(&id, "shop", "users", "").await.unwrap(),
            Some(137)
        );
        assert_eq!(
            conns
                .count(&id, "shop", "users", "{ 'address.city': 'Izmir' }")
                .await
                .unwrap(),
            Some(46)
        );
        assert!(
            conns
                .count(&id, "shop", "adults", "")
                .await
                .unwrap()
                .unwrap()
                > 0
        );

        let page = conns
            .find(
                &id,
                "shop",
                "users",
                args(
                    "{ age: { $gte: 60 } }",
                    "{ age: -1, name: 1 }",
                    "{ _id: 0, name: 1, age: 1, createdAt: 1, big: 1 }",
                    0,
                    5,
                ),
            )
            .await
            .unwrap();
        assert_eq!(page.len(), 5);
        let first = page[0].as_object().unwrap();
        // Projection order follows the stored document; types survive as canonical EJSON.
        assert_eq!(
            first.keys().collect::<Vec<_>>(),
            ["name", "age", "createdAt", "big"]
        );
        assert_eq!(first["age"], serde_json::json!({ "$numberInt": "67" }));
        assert_eq!(
            first["big"],
            serde_json::json!({ "$numberLong": "9007199254740993" })
        );
        assert!(first["createdAt"]["$date"]["$numberLong"].is_string());

        let skipped = conns
            .find(&id, "shop", "users", args("", "{ name: 1 }", "", 135, 20))
            .await
            .unwrap();
        assert_eq!(skipped.len(), 2);

        let by_date = conns
            .find(
                &id,
                "shop",
                "users",
                args(
                    "{ createdAt: { $lt: ISODate('2024-01-03') } }",
                    "",
                    "",
                    0,
                    50,
                ),
            )
            .await
            .unwrap();
        assert_eq!(by_date.len(), 2);

        let by_regex = conns
            .find(
                &id,
                "shop",
                "users",
                args("{ name: /^user1\\d$/ }", "", "", 0, 50),
            )
            .await
            .unwrap();
        assert_eq!(by_regex.len(), 10);

        let bad = conns
            .find(&id, "shop", "users", args("{ age: ", "", "", 0, 5))
            .await
            .unwrap_err();
        assert!(bad.starts_with("Filter:"), "{bad}");

        // Writes go to a scratch collection that is dropped at the end.
        let c = "humon_write_test";
        assert_eq!(
            conns
                .insert(&id, "shop", c, "[{ n: 1 }, { n: 2 }]")
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            conns
                .insert(
                    &id,
                    "shop",
                    c,
                    "{ _id: 'k', n: 3, at: ISODate('2024-05-01') }"
                )
                .await
                .unwrap(),
            1
        );
        let key = parse_id(serde_json::json!("k")).unwrap();
        let replaced = conns
            .replace(
                &id,
                "shop",
                c,
                key.clone(),
                "{ n: NumberLong(4), extra: true }",
            )
            .await
            .unwrap();
        assert_eq!(replaced["_id"], "k");
        assert_eq!(replaced["n"], serde_json::json!({ "$numberLong": "4" }));
        let stored = conns
            .get(&id, "shop", c, key.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored, replaced);
        assert!(stored.get("at").is_none(), "replace drops fields left out");
        assert_eq!(
            conns
                .replace(&id, "shop", c, key.clone(), "{ _id: 'other' }")
                .await
                .unwrap_err(),
            "_id değiştirilemez"
        );
        conns.delete(&id, "shop", c, key.clone()).await.unwrap();
        assert_eq!(
            conns.delete(&id, "shop", c, key.clone()).await.unwrap_err(),
            GONE
        );
        assert_eq!(
            conns.replace(&id, "shop", c, key, "{}").await.unwrap_err(),
            GONE
        );
        conns
            .client(&id)
            .await
            .unwrap()
            .database("shop")
            .collection::<Document>(c)
            .drop()
            .await
            .unwrap();

        conns.close(&id).await;
        assert!(conns.databases(&id).await.is_err());
    }
}
