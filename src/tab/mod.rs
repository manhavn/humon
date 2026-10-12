//! A collection tab: query bar, pager, the colored document tree, and the
//! editors for editing and inserting documents.

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::highlighter::HighlightTheme;
use gpui_kit::component::input::{Editor, EditorState, HighlightStyleResolver, Input, InputEvent, InputHighlighter, InputState};
use gpui_kit::component::{ActiveTheme, Disableable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mongodb::Client;
use serde_json::{Map, Value};

use crate::app::Humon;
use crate::history::{self, Query};
use crate::highlight::{ShellHighlighter, tokenize};
use crate::mongo::{self, CollKind, ExportFormat, FindArgs, RT};
use crate::shell::to_shell;
use crate::smart::Smart;
use rust_i18n::t;

mod editor;
mod panels;
mod status;
mod table;
mod tree;
mod view;

// Children reach each other through `use super::*`.
#[allow(unused_imports)]
use {editor::*, panels::*, status::*, table::*, tree::*, view::*};

const PAGE_SIZES: [i64; 3] = [20, 50, 100];

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Tree,
    Table,
    /// The page as text in a read-only editor: selectable, copyable, searchable.
    Json,
}

pub struct Tab {
    pub id: usize,
    pub conn_id: String,
    client: Client,
    pub db: String,
    pub coll: String,
    kind: CollKind,
    filter: Entity<InputState>,
    sort: Entity<InputState>,
    projection: Entity<InputState>,
    /// What the shown results were fetched with; paging reuses it.
    applied: Query,
    page: u64,
    page_size: i64,
    docs: Vec<Value>,
    /// None while counting; Some(None) when the count was too slow.
    count: Option<Option<u64>>,
    loading: bool,
    error: Option<String>,
    /// Server round trip of the last page load.
    took: Option<std::time::Duration>,
    /// Bumped per request so a slow, stale response can't overwrite a newer one.
    seq: u64,
    count_seq: u64,
    /// Expanded nested values, keyed by "doc index/path".
    expanded: HashSet<String>,
    /// Document index being edited.
    editing: Option<(usize, Value, DocEditor)>,
    draft: Option<DocEditor>,
    confirm_delete: Option<usize>,
    card_error: Option<(usize, String)>,
    notice: Option<String>,
    history_open: bool,
    view: ViewMode,
    json: Entity<EditorState>,
    export_open: bool,
    export_format: ExportFormat,
    /// Export every match rather than just the shown page.
    export_all: bool,
    exporting: bool,
    /// Typing helpers for filter, sort and projection.
    smart: [Smart; 3],
    _subscriptions: Vec<Subscription>,
}

impl Tab {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: usize,
        conn_id: &str,
        client: Client,
        db: &str,
        coll: &str,
        kind: CollKind,
        window: &mut Window,
        cx: &mut Context<Humon>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(t!("tab.filter_placeholder")));
        let sort = cx.new(|cx| InputState::new(window, cx).placeholder("{ _id: -1 }"));
        let projection = cx.new(|cx| InputState::new(window, cx).placeholder(t!("tab.projection_placeholder")));
        let subscriptions = [&filter, &sort, &projection]
            .into_iter()
            .enumerate()
            .map(|(which, input)| {
                cx.subscribe_in(input, window, move |this: &mut Humon, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::PressEnter { .. } => this.run_query(id, window, cx),
                    InputEvent::Change => this.query_input_changed(id, which, window, cx),
                    _ => {}
                })
            })
            .collect();
        Self {
            id,
            conn_id: conn_id.into(),
            client,
            db: db.into(),
            coll: coll.into(),
            kind,
            filter,
            sort,
            projection,
            applied: Default::default(),
            page: 0,
            page_size: 20,
            docs: vec![],
            count: None,
            loading: false,
            error: None,
            took: None,
            seq: 0,
            count_seq: 0,
            expanded: HashSet::new(),
            editing: None,
            draft: None,
            confirm_delete: None,
            card_error: None,
            notice: None,
            history_open: false,
            view: ViewMode::Tree,
            json: cx.new(|cx| crate::highlight::shell_editor(window, cx)),
            export_open: false,
            export_format: ExportFormat::Json,
            export_all: true,
            exporting: false,
            smart: Default::default(),
            _subscriptions: subscriptions,
        }
    }

    fn writable(&self) -> bool {
        self.kind != CollKind::View
    }
}

/// Which documents the current page shows (1-based, 0 when empty), and
/// whether another page follows. With the count unknown, a full page
/// suggests there is more.
#[derive(Debug, PartialEq, Eq)]
pub struct PageWindow {
    pub from: u64,
    pub to: u64,
    pub has_next: bool,
}

pub fn page_window(page: u64, page_size: i64, shown: usize, count: Option<Option<u64>>) -> PageWindow {
    let size = page_size.max(1) as u64;
    let start = page * size;
    PageWindow {
        from: if shown == 0 { 0 } else { start + 1 },
        to: start + shown as u64,
        has_next: match count {
            Some(Some(total)) => start + size < total,
            _ => shown as u64 == size,
        },
    }
}

/// The JSON view's text: the page as one shell-syntax array.
pub fn json_view_text(docs: &[Value]) -> String {
    to_shell(&Value::Array(docs.to_vec()), "")
}

fn doc_id(doc: &Value) -> Option<Value> {
    doc.get("_id").cloned()
}

impl Humon {
    fn tab_mut(&mut self, id: usize) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// Re-sets input placeholders after a language change.
    pub fn relabel_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            for (input, key) in [(&tab.filter, "tab.filter_placeholder"), (&tab.projection, "tab.projection_placeholder")] {
                input.update(cx, |s, cx| s.set_placeholder(t!(key), window, cx));
            }
        }
    }

    fn query_input_changed(&mut self, id: usize, which: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        let input = [&tab.filter, &tab.sort, &tab.projection][which].clone();
        tab.smart[which].on_change(&input, true, window, cx);
    }

    /// Applies what is typed in the query bar and goes back to the first page.
    pub fn run_query(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return };
        tab.applied = Query {
            filter: tab.filter.read(cx).value().to_string(),
            sort: tab.sort.read(cx).value().to_string(),
            projection: tab.projection.read(cx).value().to_string(),
        };
        tab.page = 0;
        let (key, query) = (history::key(&tab.conn_id, &tab.db, &tab.coll), tab.applied.clone());
        self.history.record(&key, query);
        self.persist_history();
        self.refresh(id, window, cx);
    }

    /// Puts a query into the query bar and runs it.
    fn apply_query(&mut self, id: usize, query: Query, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        tab.history_open = false;
        let inputs = [
            (tab.filter.clone(), query.filter),
            (tab.sort.clone(), query.sort),
            (tab.projection.clone(), query.projection),
        ];
        for (which, (input, text)) in inputs.into_iter().enumerate() {
            // Tell the typing helpers first, so the change isn't read as a paste.
            tab.smart[which].reset(&text);
            input.update(cx, |s, cx| s.set_value(text, window, cx));
        }
        self.run_query(id, window, cx);
    }

    fn save_favorite(&mut self, id: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return };
        let query = Query {
            filter: tab.filter.read(cx).value().to_string(),
            sort: tab.sort.read(cx).value().to_string(),
            projection: tab.projection.read(cx).value().to_string(),
        };
        let key = history::key(&tab.conn_id, &tab.db, &tab.coll);
        if self.history.add_favorite(&key, query) {
            tab.notice = Some(t!("history.saved").into());
            self.persist_history();
        }
    }

    /// Reloads the current page and the count with the applied query.
    pub fn refresh(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        tab.count = None;
        tab.count_seq += 1;
        let seq = tab.count_seq;
        let fut = mongo::count(tab.client.clone(), tab.db.clone(), tab.coll.clone(), tab.applied.filter.clone());
        self.run(fut, window, cx, move |this, res, _, _| {
            if let Some(tab) = this.tab_mut(id)
                && tab.count_seq == seq
            {
                tab.count = Some(res.ok().flatten());
            }
        });
        self.load_page(id, window, cx);
    }

    fn load_page(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        tab.seq += 1;
        tab.loading = true;
        tab.error = None;
        let seq = tab.seq;
        let args = FindArgs {
            filter: tab.applied.filter.clone(),
            sort: tab.applied.sort.clone(),
            projection: tab.applied.projection.clone(),
            skip: tab.page * tab.page_size as u64,
            limit: tab.page_size,
        };
        let fut = mongo::find(tab.client.clone(), tab.db.clone(), tab.coll.clone(), args);
        self.run(fut, window, cx, move |this, res, window, cx| {
            let Some(tab) = this.tab_mut(id) else { return };
            if tab.seq != seq {
                return;
            }
            tab.loading = false;
            match res {
                // A delete can empty the last page; step back instead of showing nothing.
                Ok(page) if page.docs.is_empty() && tab.page > 0 => {
                    tab.page -= 1;
                    this.load_page(id, window, cx);
                }
                Ok(page) => {
                    let text = json_view_text(&page.docs);
                    tab.json.update(cx, |s, cx| s.set_value(text, window, cx));
                    tab.docs = page.docs;
                    tab.took = Some(page.took);
                    tab.expanded.clear();
                    tab.confirm_delete = None;
                    tab.card_error = None;
                }
                Err(e) => {
                    tab.error = Some(e);
                    tab.docs.clear();
                    tab.took = None;
                }
            }
        });
    }

    fn go_to_page(&mut self, id: usize, page: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tab_mut(id) {
            tab.page = page;
        }
        self.load_page(id, window, cx);
    }

    fn reset_query(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_query(id, Query::default(), window, cx);
    }

    fn set_card_error(&mut self, id: usize, ix: usize, e: String) {
        if let Some(tab) = self.tab_mut(id) {
            tab.card_error = Some((ix, e));
        }
    }

    fn delete_doc(&mut self, id: usize, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        if tab.confirm_delete != Some(ix) {
            tab.confirm_delete = Some(ix);
            return;
        }
        tab.confirm_delete = None;
        let Some(doc_id) = tab.docs.get(ix).and_then(doc_id) else { return };
        let fut = mongo::delete(tab.client.clone(), tab.db.clone(), tab.coll.clone(), doc_id);
        self.run(fut, window, cx, move |this, res, window, cx| match res {
            Ok(()) => this.refresh(id, window, cx),
            Err(e) => this.set_card_error(id, ix, e),
        });
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's `test` macro, which shadows `#[test]`.
    use super::{PageWindow, json_view_text, page_window};

    #[test]
    fn json_view_shows_the_page_as_one_shell_array() {
        use serde_json::json;
        assert_eq!(json_view_text(&[]), "[]");
        let docs = vec![json!({ "_id": { "$oid": "65a1b2c3d4e5f60718293a4b" } }), json!({ "n": { "$numberInt": "1" } })];
        assert_eq!(json_view_text(&docs), "[\n  {\n    _id: ObjectId(\"65a1b2c3d4e5f60718293a4b\")\n  },\n  {\n    n: 1\n  }\n]");
    }

    #[test]
    fn page_window_bounds_and_next() {
        // Page 0 of 20, count known.
        assert_eq!(page_window(0, 20, 20, Some(Some(137))), PageWindow { from: 1, to: 20, has_next: true });
        // Last full page: exactly page_size * (page + 1) documents means no next page.
        assert_eq!(page_window(1, 20, 20, Some(Some(40))), PageWindow { from: 21, to: 40, has_next: false });
        // Partial last page.
        assert_eq!(page_window(6, 20, 17, Some(Some(137))), PageWindow { from: 121, to: 137, has_next: false });
        // Count unknown (still counting or too slow): a full page suggests more.
        assert!(page_window(0, 20, 20, None).has_next);
        assert!(page_window(0, 20, 20, Some(None)).has_next);
        assert!(!page_window(0, 20, 7, None).has_next);
        // Nothing shown.
        assert_eq!(page_window(0, 20, 0, Some(Some(0))), PageWindow { from: 0, to: 0, has_next: false });
    }
}
