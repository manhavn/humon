//! Completion for the shell editor. Database and collection names and each
//! collection's schema are fetched the first time a suggestion needs them,
//! then kept for the life of the tab.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::component::input::{CompletionProvider, Rope};
use gpui_kit::{App, Task, Window};
use lsp_types::{CompletionContext, CompletionResponse};
use mongodb::Client;

use crate::complete::shell::{Spot, complete_shell, spot};
use crate::highlight::{completion_response, is_completion_trigger};
use crate::mongo::{self, CollectionRef, RT};
use crate::schema::{Sampling, Schema};

pub struct ShellCompleter {
    inner: Rc<Inner>,
}

struct Inner {
    client: Client,
    /// The database `db` points at; the tab updates it after each run.
    db: Rc<RefCell<String>>,
    sampling: Sampling,
    databases: RefCell<Option<Rc<Vec<String>>>>,
    collections: RefCell<HashMap<String, Rc<Vec<String>>>>,
    schemas: RefCell<HashMap<(String, String), Rc<Schema>>>,
    /// Fetches under way, so typing on doesn't start the same one again.
    pending: RefCell<HashSet<String>>,
}

/// What a suggestion works from.
enum Data {
    Names(Rc<Vec<String>>),
    Schema(Rc<Schema>),
    None,
}

/// What a suggestion is waiting for.
enum Fetch {
    Databases,
    Collections(String),
    Schema(String, String),
}

impl ShellCompleter {
    pub fn new(client: Client, db: Rc<RefCell<String>>, sampling: Sampling) -> Self {
        Self {
            inner: Rc::new(Inner {
                client,
                db,
                sampling,
                databases: RefCell::default(),
                collections: RefCell::default(),
                schemas: RefCell::default(),
                pending: RefCell::default(),
            }),
        }
    }

    /// A collection's schema if a suggestion has already fetched it.
    pub fn cached_schema(&self, db: Option<&str>, coll: &str) -> Option<Rc<Schema>> {
        let db = db.map_or_else(|| self.inner.db.borrow().clone(), String::from);
        self.inner.schemas.borrow().get(&(db, coll.to_string())).cloned()
    }
}

impl Inner {
    /// The data `spot` needs if it is at hand, else what to fetch.
    fn lookup(&self, spot: &Spot) -> Result<Data, Fetch> {
        let current = self.db.borrow().clone();
        match spot {
            Spot::Databases => self.databases.borrow().clone().map(Data::Names).ok_or(Fetch::Databases),
            Spot::Collections(db) => {
                let db = db.clone().unwrap_or(current);
                let found = self.collections.borrow().get(&db).cloned();
                found.map(Data::Names).ok_or(Fetch::Collections(db))
            }
            Spot::Arg { db, coll, .. } => {
                let key = (db.clone().unwrap_or(current), coll.clone());
                let found = self.schemas.borrow().get(&key).cloned();
                found.map(Data::Schema).ok_or(Fetch::Schema(key.0, key.1))
            }
            _ => Ok(Data::None),
        }
    }
}

fn respond(text: &Rope, offset: usize, spot: &Spot, data: &Data) -> CompletionResponse {
    let src = text.to_string();
    let found = match data {
        Data::Names(names) => complete_shell(&src, offset, spot, names, None),
        Data::Schema(schema) => complete_shell(&src, offset, spot, &[], Some(schema)),
        Data::None => complete_shell(&src, offset, spot, &[], None),
    };
    completion_response(text, found)
}

impl CompletionProvider for ShellCompleter {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        if !crate::settings::suggestions_on() {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }
        let spot = spot(&text.to_string(), offset);
        let fetch = match self.inner.lookup(&spot) {
            Ok(data) => return Task::ready(Ok(respond(text, offset, &spot, &data))),
            Err(fetch) => fetch,
        };
        let key = match &fetch {
            Fetch::Databases => "dbs".to_string(),
            Fetch::Collections(db) => format!("colls:{db}"),
            Fetch::Schema(db, coll) => format!("schema:{db}.{coll}"),
        };
        if !self.inner.pending.borrow_mut().insert(key.clone()) {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }
        let (inner, text, client, sampling) = (
            self.inner.clone(),
            text.clone(),
            self.inner.client.clone(),
            self.inner.sampling,
        );
        // The fetch runs on its own: the editor drops this request's task on the
        // next keystroke, and the fetched data must still reach the cache. A
        // failed fetch is kept as empty, so it isn't retried on every keystroke.
        let (tx, rx) = futures_channel::oneshot::channel::<Data>();
        cx.spawn(async move |_| {
            let data = match fetch {
                Fetch::Databases => {
                    let found = RT.spawn(mongo::databases(client)).await;
                    let names: Vec<String> = match found {
                        Ok(Ok(dbs)) => dbs.into_iter().map(|d| d.name).collect(),
                        _ => Vec::new(),
                    };
                    let names = Rc::new(names);
                    *inner.databases.borrow_mut() = Some(names.clone());
                    Data::Names(names)
                }
                Fetch::Collections(db) => {
                    let found = RT.spawn(mongo::collections(client, db.clone())).await;
                    let names: Vec<String> = match found {
                        Ok(Ok(colls)) => colls.into_iter().map(|c| c.name).collect(),
                        _ => Vec::new(),
                    };
                    let names = Rc::new(names);
                    inner.collections.borrow_mut().insert(db, names.clone());
                    Data::Names(names)
                }
                Fetch::Schema(db, coll) => {
                    let found = RT.spawn(CollectionRef::new(client, &db, &coll).schema(sampling)).await;
                    let schema = Rc::new(match found {
                        Ok(Ok(schema)) => schema,
                        _ => Schema::default(),
                    });
                    inner.schemas.borrow_mut().insert((db, coll), schema.clone());
                    Data::Schema(schema)
                }
            };
            inner.pending.borrow_mut().remove(&key);
            let _ = tx.send(data);
        })
        .detach();
        cx.spawn(async move |_| {
            Ok(match rx.await {
                Ok(data) => respond(&text, offset, &spot, &data),
                Err(_) => CompletionResponse::Array(vec![]),
            })
        })
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        is_completion_trigger(new_text)
    }
}
