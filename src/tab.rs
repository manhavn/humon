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

const PAGE_SIZES: [i64; 3] = [20, 50, 100];
const COLUMN_WIDTH: f32 = 200.;
const ACTIONS_WIDTH: f32 = 72.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Tree,
    Table,
}

pub struct DocEditor {
    pub state: Entity<EditorState>,
    pub error: Option<String>,
    pub saving: bool,
    smart: Smart,
    _subscription: Subscription,
}

/// Which editor of a tab: the one editing a document, or the insert draft.
#[derive(Clone, Copy)]
enum EditorSlot {
    Edit,
    Draft,
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

fn doc_id(doc: &Value) -> Option<Value> {
    doc.get("_id").cloned()
}

/// An editor for shell-syntax documents in `slot` of tab `id`; ⌘↵ saves.
fn new_editor(text: String, id: usize, slot: EditorSlot, window: &mut Window, cx: &mut Context<Humon>) -> DocEditor {
    let mut smart = Smart::default();
    smart.reset(&text);
    let state = cx.new(|cx| {
        let mut state = EditorState::new(window, cx)
            .language("mongosh")
            .line_number(false)
            .folding(false)
            .default_value(text);
        state.set_highlighter_factory(
            Rc::new(|_| Some(Box::new(ShellHighlighter::default()) as Box<dyn InputHighlighter>)),
            cx,
        );
        state.focus(window, cx);
        state
    });
    let subscription = cx.subscribe_in(&state, window, move |this, input, ev: &InputEvent, window, cx| match ev {
        InputEvent::PressEnter { secondary: true, .. } => match slot {
            EditorSlot::Edit => this.save_edit(id, window, cx),
            EditorSlot::Draft => this.save_draft(id, window, cx),
        },
        InputEvent::Change => {
            let input = input.clone();
            if let Some(editor) = this.editor_mut(id, slot) {
                // The editor pairs brackets itself; only completion and wrapping run here.
                editor.smart.on_change(&input, false, window, cx);
            }
        }
        _ => {}
    });
    DocEditor { state, error: None, saving: false, smart, _subscription: subscription }
}

impl Humon {
    fn tab_mut(&mut self, id: usize) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn editor_mut(&mut self, id: usize, slot: EditorSlot) -> Option<&mut DocEditor> {
        let tab = self.tab_mut(id)?;
        match slot {
            EditorSlot::Edit => tab.editing.as_mut().map(|(_, _, e)| e),
            EditorSlot::Draft => tab.draft.as_mut(),
        }
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

    /// Asks where to save, then streams the export there on tokio.
    fn export(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return };
        if tab.exporting {
            return;
        }
        let format = tab.export_format;
        let args = FindArgs {
            filter: tab.applied.filter.clone(),
            sort: tab.applied.sort.clone(),
            projection: tab.applied.projection.clone(),
            skip: if tab.export_all { 0 } else { tab.page * tab.page_size as u64 },
            limit: if tab.export_all { 0 } else { tab.page_size },
        };
        let (client, db, coll) = (tab.client.clone(), tab.db.clone(), tab.coll.clone());
        let downloads = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Downloads");
        let picked = cx.prompt_for_new_path(&downloads, Some(&format!("{coll}.{}", format.extension())));
        tab.exporting = true;
        tab.notice = Some(t!("export.running").into());
        cx.spawn_in(window, async move |this, cx| {
            let path = match picked.await {
                Ok(Ok(Some(path))) => path,
                _ => {
                    // Dialog cancelled.
                    this.update(cx, |this, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.exporting = false;
                            tab.notice = None;
                        }
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let res = RT.spawn(mongo::export(client, db, coll, args, format, path)).await;
            this.update(cx, |this, cx| {
                if let Some(tab) = this.tab_mut(id) {
                    tab.exporting = false;
                    match res {
                        Ok(Ok(n)) => {
                            tab.notice = Some(t!("export.done", n = n, file = file).into());
                            tab.export_open = false;
                        }
                        Ok(Err(e)) => {
                            tab.notice = None;
                            tab.error = Some(e);
                        }
                        Err(e) => {
                            tab.notice = None;
                            tab.error = Some(e.to_string());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
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

    fn start_edit(&mut self, id: usize, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        let Some(doc_id) = tab.docs.get(ix).and_then(doc_id) else { return };
        tab.card_error = None;
        // Edit the full stored document: the tab may show a projection, and
        // saving a projected copy would drop the hidden fields.
        let fut = mongo::get(tab.client.clone(), tab.db.clone(), tab.coll.clone(), doc_id.clone());
        self.run(fut, window, cx, move |this, res, window, cx| {
            let text = match res {
                Ok(Some(full)) => to_shell(&full, ""),
                Ok(None) => return this.set_card_error(id, ix, mongo::gone()),
                Err(e) => return this.set_card_error(id, ix, e),
            };
            let editor = new_editor(text, id, EditorSlot::Edit, window, cx);
            if let Some(tab) = this.tab_mut(id) {
                tab.editing = Some((ix, doc_id, editor));
            }
        });
    }

    fn set_card_error(&mut self, id: usize, ix: usize, e: String) {
        if let Some(tab) = self.tab_mut(id) {
            tab.card_error = Some((ix, e));
        }
    }

    fn save_edit(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return };
        let Some((_, doc_id, editor)) = &mut tab.editing else { return };
        if editor.saving {
            return;
        }
        editor.saving = true;
        editor.error = None;
        let text = editor.state.read(cx).value().to_string();
        let fut = mongo::replace(tab.client.clone(), tab.db.clone(), tab.coll.clone(), doc_id.clone(), text);
        self.run(fut, window, cx, move |this, res, window, cx| {
            let Some(tab) = this.tab_mut(id) else { return };
            match res {
                Ok(()) => {
                    tab.editing = None;
                    this.refresh(id, window, cx);
                }
                Err(e) => {
                    if let Some((_, _, editor)) = &mut tab.editing {
                        editor.saving = false;
                        editor.error = Some(e);
                    }
                }
            }
        });
    }

    fn open_draft(&mut self, id: usize, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let editor = new_editor(text, id, EditorSlot::Draft, window, cx);
        if let Some(tab) = self.tab_mut(id) {
            tab.draft = Some(editor);
        }
    }

    fn save_draft(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return };
        let Some(editor) = &mut tab.draft else { return };
        if editor.saving {
            return;
        }
        editor.saving = true;
        editor.error = None;
        let text = editor.state.read(cx).value().to_string();
        let fut = mongo::insert(tab.client.clone(), tab.db.clone(), tab.coll.clone(), text);
        self.run(fut, window, cx, move |this, res, window, cx| {
            let Some(tab) = this.tab_mut(id) else { return };
            match res {
                Ok(n) => {
                    tab.draft = None;
                    tab.notice = Some(t!("tab.inserted", n = n).into());
                    this.refresh(id, window, cx);
                }
                Err(e) => {
                    if let Some(editor) = &mut tab.draft {
                        editor.saving = false;
                        editor.error = Some(e);
                    }
                }
            }
        });
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

    pub fn render_tab(&self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = &self.tabs[ix];
        let id = tab.id;
        let theme = cx.theme();
        let (border, muted, danger, accent, primary) = (theme.border, theme.muted_foreground, theme.danger, theme.success, theme.primary);
        let syntax = theme.highlight_theme.clone();

        let from = tab.page * tab.page_size as u64;
        let count_text = match tab.count {
            None => t!("tab.counting").to_string(),
            Some(None) => t!("tab.many").to_string(),
            Some(Some(n)) => n.to_string(),
        };
        let range = if tab.docs.is_empty() {
            if tab.loading || tab.error.is_some() { String::new() } else { t!("tab.no_docs").into() }
        } else {
            t!("tab.range", from = from + 1, to = from + tab.docs.len() as u64, total = count_text).into()
        };
        let last_page = match tab.count {
            Some(Some(n)) => Some(n.div_ceil(tab.page_size as u64).saturating_sub(1)),
            _ => None,
        };
        let has_next = match last_page {
            Some(last) => tab.page < last,
            None => tab.docs.len() as i64 == tab.page_size,
        };

        let query_bar = div()
            .flex()
            .gap_2()
            .items_end()
            .p_3()
            .border_b_1()
            .border_color(border)
            .child(labeled("Filter", Input::new(&tab.filter), muted).flex_grow(1.).flex_basis(relative(0.5)))
            .child(labeled("Sort", Input::new(&tab.sort), muted).flex_grow(1.).flex_basis(relative(0.2)))
            .child(labeled("Project", Input::new(&tab.projection), muted).flex_grow(1.).flex_basis(relative(0.2)))
            .child(Button::new("find").primary().label(t!("tab.find")).on_click(cx.listener(move |this, _, window, cx| {
                this.run_query(id, window, cx);
            })))
            .child(Button::new("reset").label(t!("tab.reset")).on_click(cx.listener(move |this, _, window, cx| {
                this.reset_query(id, window, cx);
            })))
            .child(Button::new("history").label(t!("history.toggle")).when(tab.history_open, |b| b.primary()).on_click(cx.listener(move |this, _, _, cx| {
                if let Some(tab) = this.tab_mut(id) {
                    tab.history_open = !tab.history_open;
                }
                cx.notify();
            })))
            .child(Button::new("favorite").label(t!("history.save")).on_click(cx.listener(move |this, _, _, cx| {
                this.save_favorite(id, cx);
                cx.notify();
            })));

        let page = tab.page;
        let mut toolbar = div()
            .flex()
            .gap_2()
            .items_center()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(border)
            .text_color(muted)
            .child(range)
            .when(tab.loading, |d| d.child(div().text_xs().child(t!("tab.loading").to_string())))
            .when_some(tab.notice.clone(), |d, n| d.child(div().text_xs().text_color(accent).child(n)))
            .child(div().flex_1());
        toolbar = toolbar.child(
            Button::new("export-toggle").label(t!("export.button")).when(tab.export_open, |b| b.primary()).on_click(cx.listener(move |this, _, _, cx| {
                if let Some(tab) = this.tab_mut(id) {
                    tab.export_open = !tab.export_open;
                }
                cx.notify();
            })),
        );
        if tab.writable() {
            toolbar = toolbar.child(
                Button::new("add").label(t!("tab.add")).disabled(tab.draft.is_some()).on_click(cx.listener(move |this, _, window, cx| {
                    this.open_draft(id, "{\n  \n}".into(), window, cx);
                })),
            );
        }
        for (mode, key) in [(ViewMode::Tree, "tab.view_tree"), (ViewMode::Table, "tab.view_table")] {
            toolbar = toolbar.child(
                Button::new(SharedString::from(format!("view-{key}")))
                    .label(t!(key))
                    .when(tab.view == mode, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.view = mode;
                        }
                        cx.notify();
                    })),
            );
        }
        toolbar = toolbar.child(div().w(px(8.)));
        for size in PAGE_SIZES {
            let selected = tab.page_size == size;
            toolbar = toolbar.child(
                Button::new(SharedString::from(format!("size-{size}")))
                    .label(size.to_string())
                    .when(selected, |b| b.primary())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.page_size = size;
                        }
                        this.go_to_page(id, 0, window, cx);
                    })),
            );
        }
        toolbar = toolbar
            .child(Button::new("prev").label("‹").disabled(page == 0 || tab.loading).on_click(cx.listener(move |this, _, window, cx| {
                this.go_to_page(id, page.saturating_sub(1), window, cx);
            })))
            .child(Button::new("next").label("›").disabled(!has_next || tab.loading).on_click(cx.listener(move |this, _, window, cx| {
                this.go_to_page(id, page + 1, window, cx);
            })))
            .child(Button::new("refresh").label("↻").disabled(tab.loading).on_click(cx.listener(move |this, _, window, cx| {
                this.refresh(id, window, cx);
            })));

        let mut docs = div().id(("docs", id)).flex_1().p_3().flex().flex_col().gap_2();
        // The table also scrolls sideways; the tree only needs vertical scroll.
        docs = if tab.view == ViewMode::Table { docs.overflow_scroll() } else { docs.overflow_y_scroll() };
        if let Some(draft) = &tab.draft {
            docs = docs.child(
                div()
                    .p_2()
                    .border_1()
                    .border_color(primary)
                    .rounded_md()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(t!("tab.new_document").to_string()).child(
                        div().text_xs().text_color(muted).child(t!("tab.array_hint").to_string()),
                    ))
                    .child(self.render_editor(id, draft, t!("tab.insert").into(), true, cx)),
            );
        }
        match tab.view {
            ViewMode::Tree => {
                for (i, doc) in tab.docs.iter().enumerate() {
                    docs = docs.child(self.render_card(tab, i, doc, &syntax, cx));
                }
            }
            ViewMode::Table => {
                if let Some((_, _, editor)) = &tab.editing {
                    docs = docs.child(
                        div().p_2().border_1().border_color(primary).rounded_md().child(self.render_editor(id, editor, t!("tab.save").into(), false, cx)),
                    );
                }
                if !tab.docs.is_empty() {
                    docs = docs.child(self.render_table(tab, &syntax, cx));
                }
            }
        }

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(query_bar)
            .when(tab.history_open, |d| d.child(self.render_history(tab, cx)))
            .child(toolbar)
            .when(tab.export_open, |d| d.child(self.render_export(tab, cx)))
            .when_some(tab.error.clone(), |d, e| d.child(div().px_3().py_2().text_color(danger).child(e)))
            .child(docs)
            .child(self.render_status_bar(tab, cx))
    }

    /// Top-level fields as columns; nested values show as a summary.
    fn render_table(&self, tab: &Tab, syntax: &Arc<HighlightTheme>, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, hover, panel, danger, fg) = (theme.border, theme.muted_foreground, theme.list_hover, theme.sidebar, theme.danger, theme.foreground);
        let mono = theme.mono_font_family.clone();
        let id = tab.id;
        let columns = table_columns(&tab.docs);
        let width = ACTIONS_WIDTH + COLUMN_WIDTH * columns.len() as f32;
        let writable = tab.writable();

        let cell = |w: f32| div().flex_none().w(px(w)).px_2().py_1().border_r_1().border_color(border).truncate();
        let mut header = div().flex().flex_none().w(px(width)).bg(panel).border_b_1().border_color(border).font_weight(FontWeight::SEMIBOLD).child(cell(ACTIONS_WIDTH));
        for c in &columns {
            header = header.child(cell(COLUMN_WIDTH).text_color(fg).child(c.clone()));
        }
        let mut table = div().flex().flex_col().flex_none().w(px(width)).border_1().border_color(border).rounded_md().font_family(mono).text_xs().child(header);

        for (ix, doc) in tab.docs.iter().enumerate() {
            let confirm = tab.confirm_delete == Some(ix);
            let has_id = doc.get("_id").is_some();
            let icon = |key: &str, label: &'static str| {
                div().id(SharedString::from(format!("t{key}-{id}-{ix}"))).px_1().rounded_sm().cursor_pointer().hover(|s| s.bg(hover)).child(label)
            };
            let mut actions = div().flex().gap_1().text_color(muted);
            if writable && has_id {
                actions = actions.child(icon("edit", "✎").on_click(cx.listener(move |this, _, window, cx| this.start_edit(id, ix, window, cx))));
            }
            let shell = to_shell(doc, "");
            actions = actions.child(icon("copy", "⧉").on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(shell.clone()))));
            if writable && has_id {
                actions = actions.child(
                    icon("delete", if confirm { "?" } else { "🗑" })
                        .when(confirm, |d| d.text_color(danger))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.delete_doc(id, ix, window, cx);
                            cx.notify();
                        })),
                );
            }
            let mut row = div()
                .flex()
                .flex_none()
                .w(px(width))
                .border_b_1()
                .border_color(border)
                .hover(|s| s.bg(hover))
                .child(cell(ACTIONS_WIDTH).child(actions));
            for c in &columns {
                let content = match doc.get(c) {
                    None => div(),
                    Some(Value::Object(m)) if !is_typed(m) => div().text_color(muted).child(t!("tree.fields", n = m.len()).to_string()),
                    Some(Value::Array(a)) => div().text_color(muted).child(t!("tree.array", n = a.len()).to_string()),
                    Some(v) => {
                        // One line per cell: newlines would break the row height.
                        let text = to_shell(v, "").replace('\n', "⏎");
                        let highlights: Vec<_> = tokenize(&text).into_iter().filter_map(|(r, k)| syntax.style(k).map(|s| (r, s))).collect();
                        div().child(StyledText::new(text).with_highlights(highlights))
                    }
                };
                row = row.child(cell(COLUMN_WIDTH).child(content));
            }
            table = table.child(row);
            if let Some((i, e)) = &tab.card_error
                && *i == ix
            {
                table = table.child(div().px_2().py_1().text_color(danger).child(e.clone()));
            }
        }
        table
    }

    /// Format and scope choice for exporting the current query.
    fn render_export(&self, tab: &Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, panel) = (theme.border, theme.muted_foreground, theme.sidebar);
        let id = tab.id;
        let mut row = div().flex().flex_wrap().items_center().gap_2().px_3().py_2().border_b_1().border_color(border).bg(panel).text_xs();
        for (format, label) in [(ExportFormat::Json, "JSON"), (ExportFormat::Csv, "CSV")] {
            row = row.child(
                Button::new(SharedString::from(format!("format-{label}")))
                    .label(label)
                    .when(tab.export_format == format, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.export_format = format;
                        }
                        cx.notify();
                    })),
            );
        }
        let all_label = match tab.count {
            Some(Some(n)) => t!("export.all_count", n = thousands(n)).to_string(),
            _ => t!("export.all").to_string(),
        };
        row = row.child(div().w(px(12.)));
        for (all, label) in [(false, t!("export.page", n = tab.docs.len()).to_string()), (true, all_label)] {
            row = row.child(
                Button::new(SharedString::from(format!("scope-{all}")))
                    .label(label)
                    .when(tab.export_all == all, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.export_all = all;
                        }
                        cx.notify();
                    })),
            );
        }
        row.child(div().flex_1())
            .when(tab.export_format == ExportFormat::Csv, |d| d.child(div().text_color(muted).child(t!("export.csv_note").to_string())))
            .child(
                Button::new("export-save")
                    .primary()
                    .label(if tab.exporting { t!("export.running") } else { t!("export.save") })
                    .disabled(tab.exporting)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.export(id, window, cx);
                        cx.notify();
                    })),
            )
    }

    /// Favorites first, then recent queries; clicking one runs it.
    fn render_history(&self, tab: &Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, hover, panel, warning) = (theme.border, theme.muted_foreground, theme.list_hover, theme.sidebar, theme.warning);
        let mono = theme.mono_font_family.clone();
        let id = tab.id;
        let key = history::key(&tab.conn_id, &tab.db, &tab.coll);
        let favorites = self.history.favorites(&key);
        let recent = self.history.recent(&key);
        let row = |prefix: String, query: &Query, icon: &'static str, color: Hsla| {
            let q = query.clone();
            div()
                .id(SharedString::from(format!("{prefix}-{id}")))
                .flex()
                .gap_2()
                .items_center()
                .px_3()
                .py_1()
                .cursor_pointer()
                .hover(|s| s.bg(hover))
                .child(div().w(px(14.)).text_color(color).child(icon))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().font_family(mono.clone()).child(query.summary()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.apply_query(id, q.clone(), window, cx);
                    cx.notify();
                }))
        };
        let mut list = div().id(("history", id)).flex().flex_col().max_h(px(260.)).overflow_y_scroll().border_b_1().border_color(border).bg(panel).text_xs();
        if favorites.is_empty() && recent.is_empty() {
            return list.child(div().px_3().py_2().text_color(muted).child(t!("history.empty").to_string()));
        }
        if !favorites.is_empty() {
            list = list.child(div().px_3().pt_2().pb_1().text_color(muted).font_weight(FontWeight::SEMIBOLD).child(t!("history.favorites").to_string()));
            for (i, fav) in favorites.iter().enumerate() {
                let k = key.clone();
                list = list.child(row(format!("fav-{i}"), fav, "★", warning).child(
                    div()
                        .id(SharedString::from(format!("unfav-{i}-{id}")))
                        .px_1()
                        .text_color(muted)
                        .child("×")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.history.remove_favorite(&k, i);
                            this.persist_history();
                            cx.notify();
                        })),
                ));
            }
        }
        if !recent.is_empty() {
            list = list.child(div().px_3().pt_2().pb_1().text_color(muted).font_weight(FontWeight::SEMIBOLD).child(t!("history.recent").to_string()));
            for (i, q) in recent.iter().enumerate() {
                list = list.child(row(format!("recent-{i}"), q, "↺", muted));
            }
        }
        list
    }

    /// Bottom bar, as in Compass: total count, query time, and where you are.
    fn render_status_bar(&self, tab: &Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, panel) = (theme.border, theme.muted_foreground, theme.sidebar);
        let conn = self.conns.iter().find(|c| c.saved.id == tab.conn_id);
        let count = match tab.count {
            None => t!("status.counting").to_string(),
            Some(None) => t!("status.count_slow").to_string(),
            Some(Some(n)) => t!("status.documents", n = thousands(n)).to_string(),
        };
        let took = match (tab.loading, tab.took) {
            (true, _) => t!("status.running").to_string(),
            (false, Some(d)) => t!("status.took", time = format_duration(d)).to_string(),
            (false, None) => String::new(),
        };
        let filtered = !tab.applied.filter.trim().is_empty();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .px_3()
            .h(px(26.))
            .border_t_1()
            .border_color(border)
            .bg(panel)
            .text_xs()
            .text_color(muted)
            .child(count)
            .when(filtered, |d| d.child(t!("status.filtered").to_string()))
            .when(!took.is_empty(), |d| d.child("·").child(took))
            .child(div().flex_1())
            .when_some(conn, |d, c| {
                d.when_some(c.color(), |d, color| d.child(div().size(px(7.)).rounded_full().bg(color)))
                    .child(format!("{} · {}.{}", c.saved.name, tab.db, tab.coll))
            })
    }

    fn render_editor(&self, id: usize, editor: &DocEditor, save_label: SharedString, is_draft: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let lines = editor.state.read(cx).value().lines().count().clamp(6, 30);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(Editor::new(&editor.state).h(px(lines as f32 * 18. + 16.)))
            .when_some(editor.error.clone(), |d, e| d.child(div().text_xs().text_color(theme.danger).child(e)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().text_xs().text_color(theme.muted_foreground).child(t!("tab.save_hint").to_string()))
                    .child(Button::new(("cancel-edit", id)).label(t!("tab.cancel")).on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            if is_draft { tab.draft = None } else { tab.editing = None }
                        }
                        cx.notify();
                    })))
                    .child(
                        Button::new(("save-edit", id))
                            .primary()
                            .label(if editor.saving { t!("tab.saving").into() } else { save_label })
                            .disabled(editor.saving)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if is_draft { this.save_draft(id, window, cx) } else { this.save_edit(id, window, cx) }
                            })),
                    ),
            )
    }

    fn render_card(&self, tab: &Tab, ix: usize, doc: &Value, syntax: &Arc<HighlightTheme>, cx: &mut Context<Self>) -> AnyElement {
        let id = tab.id;
        let theme = cx.theme();
        let (border, muted, danger, hover, bg) = (theme.border, theme.muted_foreground, theme.danger, theme.list_hover, theme.background);
        let (primary, mono) = (theme.primary, theme.mono_font_family.clone());

        if let Some((edit_ix, _, editor)) = &tab.editing
            && *edit_ix == ix
        {
            return div()
                .p_2()
                .border_1()
                .border_color(primary)
                .rounded_md()
                .child(self.render_editor(id, editor, t!("tab.save").into(), false, cx))
                .into_any_element();
        }

        let writable = tab.writable() && doc.get("_id").is_some();
        let confirm = tab.confirm_delete == Some(ix);
        let group: SharedString = format!("card-{id}-{ix}").into();
        let action = |label: String, key: &str| {
            div()
                .id(SharedString::from(format!("{key}-{id}-{ix}")))
                .px_2()
                .border_1()
                .border_color(border)
                .rounded_sm()
                .bg(bg)
                .text_xs()
                .cursor_pointer()
                .hover(|s| s.bg(hover))
                .child(label)
        };
        let mut actions = div()
            .absolute()
            .top_1()
            .right_2()
            .flex()
            .gap_1()
            .text_color(muted)
            .when(!confirm, |d| d.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.)));
        if writable {
            actions = actions
                .child(action(t!("card.edit").into(), "edit").on_click(cx.listener(move |this, _, window, cx| this.start_edit(id, ix, window, cx))))
                .child(action(t!("card.clone").into(), "clone").on_click(cx.listener(move |this, _, window, cx| {
                    let Some(doc) = this.tab_mut(id).and_then(|t| t.docs.get(ix).cloned()) else { return };
                    let mut copy = doc.as_object().cloned().unwrap_or_default();
                    copy.remove("_id");
                    this.open_draft(id, to_shell(&Value::Object(copy), ""), window, cx);
                })));
        }
        let shell = to_shell(doc, "");
        actions = actions.child(action(t!("card.copy").into(), "copy").on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(shell.clone()));
        }));
        if writable {
            actions = actions.child(
                action(if confirm { t!("card.confirm_delete") } else { t!("card.delete") }.into(), "delete")
                    .when(confirm, |d| d.text_color(danger).border_color(danger))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.delete_doc(id, ix, window, cx);
                        cx.notify();
                    })),
            );
        }

        let mut rows = Vec::new();
        if let Value::Object(map) = doc {
            for (k, v) in map {
                self.push_rows(&mut rows, tab, &format!("{ix}"), k, v, 0, syntax, cx);
            }
        }
        div()
            .id(SharedString::from(format!("card-{id}-{ix}")))
            .group(group)
            .relative()
            .p_2()
            .border_1()
            .border_color(border)
            .rounded_md()
            .font_family(mono)
            .text_xs()
            .children(rows)
            .child(actions)
            .when_some(tab.card_error.as_ref().filter(|(i, _)| *i == ix).map(|(_, e)| e.clone()), |d, e| {
                d.child(div().pt_1().text_color(danger).child(e))
            })
            .into_any_element()
    }

    /// One row per field; nested objects and arrays start collapsed.
    #[allow(clippy::too_many_arguments)]
    fn push_rows(&self, rows: &mut Vec<AnyElement>, tab: &Tab, parent: &str, key: &str, value: &Value, depth: usize, syntax: &Arc<HighlightTheme>, cx: &mut Context<Self>) {
        let theme = cx.theme();
        let (muted, hover) = (theme.muted_foreground, theme.list_hover);
        let path = format!("{parent}/{key}");
        let container = match value {
            Value::Object(m) if !is_typed(m) => Some(if m.is_empty() { "{}".to_string() } else { t!("tree.fields", n = m.len()).into() }),
            Value::Array(a) => Some(t!("tree.array", n = a.len()).into()),
            _ => None,
        };
        let key_color = syntax.style("property").and_then(|s| s.color).unwrap_or(theme.foreground);
        let row = div().flex().gap_1().pl(px(depth as f32 * 16.)).min_h(px(18.)).items_center().whitespace_nowrap();
        let Some(summary) = container else {
            let text = to_shell(value, "");
            let highlights: Vec<_> = tokenize(&text)
                .into_iter()
                .filter_map(|(r, kind)| syntax.style(kind).map(|s| (r, s)))
                .collect();
            rows.push(
                row.child(div().w(px(10.)))
                    .child(div().text_color(key_color).child(format!("{key}:")))
                    .child(StyledText::new(text).with_highlights(highlights))
                    .into_any_element(),
            );
            return;
        };
        let open = tab.expanded.contains(&path);
        let id = tab.id;
        rows.push(
            row.id(SharedString::from(format!("row-{id}-{path}")))
                .cursor_pointer()
                .hover(|s| s.bg(hover))
                .on_click(cx.listener({
                    let path = path.clone();
                    move |this, _, _, cx| {
                        if let Some(tab) = this.tab_mut(id)
                            && !tab.expanded.remove(&path)
                        {
                            tab.expanded.insert(path.clone());
                        }
                        cx.notify();
                    }
                }))
                .child(div().w(px(10.)).text_color(muted).child(if open { "▾" } else { "▸" }))
                .child(div().text_color(key_color).child(format!("{key}:")))
                .child(div().text_color(muted).child(summary))
                .into_any_element(),
        );
        if !open {
            return;
        }
        match value {
            Value::Object(m) => {
                for (k, v) in m {
                    self.push_rows(rows, tab, &path, k, v, depth + 1, syntax, cx);
                }
            }
            Value::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    self.push_rows(rows, tab, &path, &i.to_string(), v, depth + 1, syntax, cx);
                }
            }
            _ => {}
        }
    }
}

/// Whether an object is an extended JSON wrapper like `{"$oid": …}`, shown as one value.
fn is_typed(m: &Map<String, Value>) -> bool {
    m.len() <= 2 && m.keys().next().is_some_and(|k| k.starts_with('$'))
}

fn labeled(label: &'static str, input: Input, muted: Hsla) -> Div {
    div().flex().flex_col().gap_1().min_w_0().child(div().text_xs().text_color(muted).child(label)).child(input)
}

/// 1234567 -> "1.234.567", as Turkish readers expect.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        "<1 ms".into()
    } else if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.2} s", ms / 1000.0).replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's `test` macro, which shadows `#[test]`.
    use super::{format_duration, table_columns, thousands};
    use std::time::Duration;

    #[test]
    fn table_columns_keep_first_seen_order_with_id_first() {
        let docs = vec![
            serde_json::json!({ "name": "a", "_id": 1 }),
            serde_json::json!({ "age": 3, "name": "b" }),
        ];
        assert_eq!(table_columns(&docs), ["_id", "name", "age"]);
    }

    #[test]
    fn formats_counts_and_durations() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(1234), "1.234");
        assert_eq!(thousands(1234567), "1.234.567");
        assert_eq!(format_duration(Duration::from_micros(300)), "<1 ms");
        assert_eq!(format_duration(Duration::from_millis(42)), "42 ms");
        assert_eq!(format_duration(Duration::from_millis(1530)), "1,53 s");
    }
}

/// Column order for the table: `_id` first, then fields in the order they first appear.
fn table_columns(docs: &[Value]) -> Vec<String> {
    let mut columns: Vec<String> = Vec::new();
    for doc in docs {
        if let Value::Object(m) = doc {
            for k in m.keys() {
                if !columns.iter().any(|c| c == k) {
                    columns.push(k.clone());
                }
            }
        }
    }
    if let Some(pos) = columns.iter().position(|c| c == "_id") {
        let id = columns.remove(pos);
        columns.insert(0, id);
    }
    columns
}
