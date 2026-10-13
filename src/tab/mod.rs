//! A collection tab: query bar, pager, the colored document tree, and the
//! editors for editing and inserting documents.

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::highlighter::HighlightTheme;
use gpui_kit::component::input::{Editor, EditorState, HighlightStyleResolver, InputEvent, InputHighlighter};
use gpui_kit::component::{ActiveTheme, Disableable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::Value;

use crate::app::Humon;
use crate::complete::Mode;
use crate::complete::check::check;
use crate::highlight::{ShellHighlighter, show_problems, tokenize};
use crate::history::{self, Query};
use crate::mongo::{CollKind, CollectionRef, ExportFormat, FindArgs, RT};
use crate::shell::{is_wrapper, to_shell};
use crate::smart::Smart;
use rust_i18n::t;

mod editor;
mod panels;
mod query_editor;
mod schema_view;
pub(crate) mod status;
mod table;
mod tree;
mod view;

// Children reach each other through `use super::*`.
#[allow(unused_imports)]
use {editor::*, panels::*, query_editor::*, status::*, table::*, tree::*, view::*};

const PAGE_SIZES: [i64; 3] = [20, 50, 100];

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Tree,
    Table,
    /// The page as text in a read-only editor: selectable, copyable, searchable.
    Json,
    /// The collection's fields, from the sampled schema.
    Schema,
}

pub struct Tab {
    pub id: usize,
    pub conn_id: String,
    /// The collection this tab shows; renaming it updates `target.name`.
    pub target: CollectionRef,
    kind: CollKind,
    filter: Entity<EditorState>,
    sort: Entity<EditorState>,
    projection: Entity<EditorState>,
    /// Sampled field names and types, for completion; None until sampled.
    schema: SchemaCell,
    /// What the shown results were fetched with; paging reuses it.
    applied: Query,
    page: u64,
    page_size: i64,
    docs: Vec<Value>,
    count: Count,
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
    /// The query as driver code, in a panel under the query bar.
    code_open: bool,
    code_lang: crate::codegen::Lang,
    code_view: Entity<EditorState>,
    code_error: Option<String>,
    indexes_open: bool,
    /// None while loading.
    indexes: Option<Result<Vec<crate::mongo::IndexInfo>, String>>,
    index_keys: Entity<EditorState>,
    index_unique: bool,
    index_error: Option<String>,
    /// Index whose Drop was clicked once.
    confirm_drop_index: Option<String>,
    /// The last explain of the query bar, shown under it.
    explain: Option<Result<crate::explain::Summary, String>>,
    explaining: bool,
    /// Typing helpers for filter, sort and projection.
    smart: [Smart; 3],
    _subscriptions: Vec<Subscription>,
}

impl Tab {
    pub fn new(
        id: usize,
        conn_id: &str,
        target: CollectionRef,
        kind: CollKind,
        window: &mut Window,
        cx: &mut Context<Humon>,
    ) -> Self {
        let schema = SchemaCell::default();
        let filter = cx
            .new(|cx| query_editor(Mode::Filter, schema.clone(), window, cx).placeholder(t!("tab.filter_placeholder")));
        let sort = cx.new(|cx| query_editor(Mode::Sort, schema.clone(), window, cx).placeholder("{ _id: -1 }"));
        let projection = cx.new(|cx| {
            query_editor(Mode::Project, schema.clone(), window, cx).placeholder(t!("tab.projection_placeholder"))
        });
        // Index keys read like a sort: fields valued 1 or -1, with completion.
        let index_keys = cx.new(|cx| query_editor(Mode::Sort, schema.clone(), window, cx).placeholder("{ field: 1 }"));
        let subscriptions = [&filter, &sort, &projection]
            .into_iter()
            .enumerate()
            .map(|(which, input)| {
                cx.subscribe_in(
                    input,
                    window,
                    move |this: &mut Humon, _, ev: &InputEvent, window, cx| match ev {
                        InputEvent::PressEnter { .. } => this.run_query(id, window, cx),
                        InputEvent::Change => this.query_input_changed(id, which, window, cx),
                        _ => {}
                    },
                )
            })
            .collect();
        Self {
            id,
            conn_id: conn_id.into(),
            target,
            kind,
            filter,
            sort,
            projection,
            schema,
            applied: Default::default(),
            page: 0,
            page_size: 20,
            docs: vec![],
            count: Count::Counting,
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
            code_open: false,
            code_lang: crate::codegen::Lang::Node,
            code_view: cx.new(|cx| crate::highlight::shell_editor(window, cx)),
            code_error: None,
            indexes_open: false,
            indexes: None,
            index_keys,
            index_unique: false,
            index_error: None,
            confirm_drop_index: None,
            explain: None,
            explaining: false,
            smart: Default::default(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows a new page in every view at once, so none is left stale.
    fn set_docs(
        &mut self,
        docs: Vec<Value>,
        took: Option<std::time::Duration>,
        window: &mut Window,
        cx: &mut Context<Humon>,
    ) {
        let text = if docs.is_empty() {
            String::new()
        } else {
            json_view_text(&docs)
        };
        self.json.update(cx, |s, cx| s.set_value(text, window, cx));
        self.docs = docs;
        self.took = took;
        self.expanded.clear();
        self.confirm_delete = None;
        self.card_error = None;
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

/// The total for the pager and status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Count {
    Counting,
    Known(u64),
    /// An exact count would have taken too long (server time limit).
    TooSlow,
    Failed(String),
}

impl Count {
    pub fn from_result(result: Result<Option<u64>, String>) -> Self {
        match result {
            Ok(Some(n)) => Self::Known(n),
            Ok(None) => Self::TooSlow,
            Err(e) => Self::Failed(e),
        }
    }
}

pub fn page_window(page: u64, page_size: i64, shown: usize, count: &Count) -> PageWindow {
    let size = page_size.max(1) as u64;
    let start = page * size;
    PageWindow {
        from: if shown == 0 { 0 } else { start + 1 },
        to: start + shown as u64,
        has_next: match count {
            Count::Known(total) => start + size < *total,
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
            for (input, key) in [
                (&tab.filter, "tab.filter_placeholder"),
                (&tab.projection, "tab.projection_placeholder"),
            ] {
                input.update(cx, |s, cx| s.set_placeholder(t!(key), window, cx));
            }
        }
    }

    fn query_input_changed(&mut self, id: usize, which: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        let input = [&tab.filter, &tab.sort, &tab.projection][which].clone();
        // A pasted or Shift+Enter line break: fold it; the change comes back here.
        if let Some(flat) = one_line(&input.read(cx).value()) {
            input.update(cx, |s, cx| s.set_value(flat, window, cx));
            return;
        }
        tab.smart[which].on_change(&input, true, window, cx);
        self.check_query_box(id, which, cx);
        self.refresh_code(id, window, cx);
    }

    /// Underlines problems in a query box against the sampled schema.
    fn check_query_box(&mut self, id: usize, which: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        let input = [&tab.filter, &tab.sort, &tab.projection][which].clone();
        let mode = [Mode::Filter, Mode::Sort, Mode::Project][which];
        let schema = tab.schema.borrow().clone().unwrap_or_default();
        let text = input.read(cx).value().to_string();
        let problems = check(&text, mode, &schema);
        input.update(cx, |s, cx| show_problems(s, problems, cx));
    }

    /// Applies what is typed in the query bar and goes back to the first page.
    pub fn run_query(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        tab.applied = Query {
            filter: tab.filter.read(cx).value().to_string(),
            sort: tab.sort.read(cx).value().to_string(),
            projection: tab.projection.read(cx).value().to_string(),
        };
        tab.page = 0;
        let (key, query) = (
            history::key(&tab.conn_id, &tab.target.db, &tab.target.name),
            tab.applied.clone(),
        );
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
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let query = Query {
            filter: tab.filter.read(cx).value().to_string(),
            sort: tab.sort.read(cx).value().to_string(),
            projection: tab.projection.read(cx).value().to_string(),
        };
        let key = history::key(&tab.conn_id, &tab.target.db, &tab.target.name);
        if self.history.add_favorite(&key, query) {
            tab.notice = Some(t!("history.saved").into());
            self.persist_history();
        }
    }

    /// Samples the collection in the background so the query boxes can suggest
    /// its fields. Failing (no permission, say) just leaves completion off.
    pub fn load_schema(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let sampling = self.settings.sampling();
        let Some(tab) = self.tab_mut(id) else { return };
        let fut = tab.target.clone().schema(sampling);
        self.run(fut, window, cx, move |this, res, _, cx| {
            if let (Some(tab), Ok(schema)) = (this.tab_mut(id), res) {
                *tab.schema.borrow_mut() = Some(Rc::new(schema));
                for which in 0..3 {
                    this.check_query_box(id, which, cx);
                }
            }
        });
    }

    /// Reloads the current page and the count with the applied query.
    pub fn refresh(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        tab.count = Count::Counting;
        tab.count_seq += 1;
        let seq = tab.count_seq;
        let fut = tab.target.clone().count(tab.applied.filter.clone());
        self.run(fut, window, cx, move |this, res, _, _| {
            if let Some(tab) = this.tab_mut(id)
                && tab.count_seq == seq
            {
                tab.count = Count::from_result(res);
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
        let fut = tab.target.clone().find(args);
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
                Ok(page) => tab.set_docs(page.docs, Some(page.took), window, cx),
                Err(e) => {
                    tab.error = Some(e);
                    tab.set_docs(Vec::new(), None, window, cx);
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
        let Some(doc_id) = tab.docs.get(ix).and_then(doc_id) else {
            return;
        };
        let fut = tab.target.clone().delete(doc_id);
        self.run(fut, window, cx, move |this, res, window, cx| match res {
            Ok(()) => this.refresh(id, window, cx),
            Err(e) => this.set_card_error(id, ix, e),
        });
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's `test` macro, which shadows `#[test]`.
    use super::{Count, PageWindow, json_view_text, page_window};

    #[test]
    fn json_view_shows_the_page_as_one_shell_array() {
        use serde_json::json;
        assert_eq!(json_view_text(&[]), "[]");
        let docs = vec![
            json!({ "_id": { "$oid": "65a1b2c3d4e5f60718293a4b" } }),
            json!({ "n": { "$numberInt": "1" } }),
        ];
        assert_eq!(
            json_view_text(&docs),
            "[\n  {\n    _id: ObjectId(\"65a1b2c3d4e5f60718293a4b\")\n  },\n  {\n    n: 1\n  }\n]"
        );
    }

    #[test]
    fn page_window_bounds_and_next() {
        // Page 0 of 20, count known.
        assert_eq!(
            page_window(0, 20, 20, &Count::Known(137)),
            PageWindow {
                from: 1,
                to: 20,
                has_next: true
            }
        );
        // Last full page: exactly page_size * (page + 1) documents means no next page.
        assert_eq!(
            page_window(1, 20, 20, &Count::Known(40)),
            PageWindow {
                from: 21,
                to: 40,
                has_next: false
            }
        );
        // Partial last page.
        assert_eq!(
            page_window(6, 20, 17, &Count::Known(137)),
            PageWindow {
                from: 121,
                to: 137,
                has_next: false
            }
        );
        // Count unknown (still counting, too slow, or failed): a full page suggests more.
        for count in [Count::Counting, Count::TooSlow, Count::Failed("boom".into())] {
            assert!(page_window(0, 20, 20, &count).has_next, "{count:?}");
            assert!(!page_window(0, 20, 7, &count).has_next, "{count:?}");
        }
        // Nothing shown.
        assert_eq!(
            page_window(0, 20, 0, &Count::Known(0)),
            PageWindow {
                from: 0,
                to: 0,
                has_next: false
            }
        );
    }

    #[test]
    fn a_failed_count_is_not_reported_as_slow() {
        assert_eq!(Count::from_result(Ok(Some(5))), Count::Known(5));
        assert_eq!(Count::from_result(Ok(None)), Count::TooSlow);
        assert_eq!(
            Count::from_result(Err("bad filter".into())),
            Count::Failed("bad filter".into())
        );
    }
}
