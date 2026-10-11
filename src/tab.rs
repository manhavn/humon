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
use crate::highlight::{ShellHighlighter, tokenize};
use crate::mongo::{self, CollKind, FindArgs};
use crate::shell::to_shell;
use crate::smart::Smart;

const PAGE_SIZES: [i64; 3] = [20, 50, 100];

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
    applied: (String, String, String),
    page: u64,
    page_size: i64,
    docs: Vec<Value>,
    /// None while counting; Some(None) when the count was too slow.
    count: Option<Option<u64>>,
    loading: bool,
    error: Option<String>,
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
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("{ alan: 'değer' }"));
        let sort = cx.new(|cx| InputState::new(window, cx).placeholder("{ _id: -1 }"));
        let projection = cx.new(|cx| InputState::new(window, cx).placeholder("{ alan: 1 }"));
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
            seq: 0,
            count_seq: 0,
            expanded: HashSet::new(),
            editing: None,
            draft: None,
            confirm_delete: None,
            card_error: None,
            notice: None,
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

    fn query_input_changed(&mut self, id: usize, which: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        let input = [&tab.filter, &tab.sort, &tab.projection][which].clone();
        tab.smart[which].on_change(&input, true, window, cx);
    }

    /// Applies what is typed in the query bar and goes back to the first page.
    pub fn run_query(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return };
        tab.applied = (
            tab.filter.read(cx).value().to_string(),
            tab.sort.read(cx).value().to_string(),
            tab.projection.read(cx).value().to_string(),
        );
        tab.page = 0;
        self.refresh(id, window, cx);
    }

    /// Reloads the current page and the count with the applied query.
    pub fn refresh(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        tab.count = None;
        tab.count_seq += 1;
        let seq = tab.count_seq;
        let fut = mongo::count(tab.client.clone(), tab.db.clone(), tab.coll.clone(), tab.applied.0.clone());
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
            filter: tab.applied.0.clone(),
            sort: tab.applied.1.clone(),
            projection: tab.applied.2.clone(),
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
                Ok(docs) if docs.is_empty() && tab.page > 0 => {
                    tab.page -= 1;
                    this.load_page(id, window, cx);
                }
                Ok(docs) => {
                    tab.docs = docs;
                    tab.expanded.clear();
                    tab.confirm_delete = None;
                    tab.card_error = None;
                }
                Err(e) => {
                    tab.error = Some(e);
                    tab.docs.clear();
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
        let Some(tab) = self.tab_mut(id) else { return };
        let inputs = [tab.filter.clone(), tab.sort.clone(), tab.projection.clone()];
        for input in inputs {
            input.update(cx, |s, cx| s.set_value("", window, cx));
        }
        self.run_query(id, window, cx);
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
                Ok(None) => return this.set_card_error(id, ix, mongo::GONE.into()),
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
                    tab.notice = Some(format!("{n} doküman eklendi"));
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
        let (border, muted, danger, accent) = (theme.border, theme.muted_foreground, theme.danger, theme.success);
        let syntax = theme.highlight_theme.clone();

        let from = tab.page * tab.page_size as u64;
        let count_text = match tab.count {
            None => "sayılıyor…".to_string(),
            Some(None) => "çok".to_string(),
            Some(Some(n)) => n.to_string(),
        };
        let range = if tab.docs.is_empty() {
            if tab.loading || tab.error.is_some() { String::new() } else { "Doküman yok".into() }
        } else {
            format!("{}–{} / {count_text}", from + 1, from + tab.docs.len() as u64)
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
            .child(Button::new("find").primary().label("Bul").on_click(cx.listener(move |this, _, window, cx| {
                this.run_query(id, window, cx);
            })))
            .child(Button::new("reset").label("Sıfırla").on_click(cx.listener(move |this, _, window, cx| {
                this.reset_query(id, window, cx);
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
            .when(tab.loading, |d| d.child(div().text_xs().child("Yükleniyor…")))
            .when_some(tab.notice.clone(), |d, n| d.child(div().text_xs().text_color(accent).child(n)))
            .child(div().flex_1());
        if tab.writable() {
            toolbar = toolbar.child(
                Button::new("add").label("+ Ekle").disabled(tab.draft.is_some()).on_click(cx.listener(move |this, _, window, cx| {
                    this.open_draft(id, "{\n  \n}".into(), window, cx);
                })),
            );
        }
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

        let mut docs = div().id(("docs", id)).flex_1().overflow_y_scroll().p_3().flex().flex_col().gap_2();
        if let Some(draft) = &tab.draft {
            docs = docs.child(
                div()
                    .p_2()
                    .border_1()
                    .border_color(theme.primary)
                    .rounded_md()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Yeni doküman").child(
                        div().text_xs().text_color(muted).child("Birden fazla için dizi yaz: [{…}, {…}]"),
                    ))
                    .child(self.render_editor(id, draft, "Ekle", true, cx)),
            );
        }
        for (i, doc) in tab.docs.iter().enumerate() {
            docs = docs.child(self.render_card(tab, i, doc, &syntax, cx));
        }

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(query_bar)
            .child(toolbar)
            .when_some(tab.error.clone(), |d, e| d.child(div().px_3().py_2().text_color(danger).child(e)))
            .child(docs)
    }

    fn render_editor(&self, id: usize, editor: &DocEditor, save_label: &'static str, is_draft: bool, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .child(div().flex_1().text_xs().text_color(theme.muted_foreground).child("⌘↵ kaydet"))
                    .child(Button::new(("cancel-edit", id)).label("Vazgeç").on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            if is_draft { tab.draft = None } else { tab.editing = None }
                        }
                        cx.notify();
                    })))
                    .child(
                        Button::new(("save-edit", id))
                            .primary()
                            .label(if editor.saving { "Kaydediliyor…" } else { save_label })
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
                .child(self.render_editor(id, editor, "Kaydet", false, cx))
                .into_any_element();
        }

        let writable = tab.writable() && doc.get("_id").is_some();
        let confirm = tab.confirm_delete == Some(ix);
        let group: SharedString = format!("card-{id}-{ix}").into();
        let action = |label: &'static str, key: &str| {
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
                .child(action("Düzenle", "edit").on_click(cx.listener(move |this, _, window, cx| this.start_edit(id, ix, window, cx))))
                .child(action("Klonla", "clone").on_click(cx.listener(move |this, _, window, cx| {
                    let Some(doc) = this.tab_mut(id).and_then(|t| t.docs.get(ix).cloned()) else { return };
                    let mut copy = doc.as_object().cloned().unwrap_or_default();
                    copy.remove("_id");
                    this.open_draft(id, to_shell(&Value::Object(copy), ""), window, cx);
                })));
        }
        let shell = to_shell(doc, "");
        actions = actions.child(action("Kopyala", "copy").on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(shell.clone()));
        }));
        if writable {
            actions = actions.child(
                action(if confirm { "Emin misin?" } else { "Sil" }, "delete")
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
            Value::Object(m) if !is_typed(m) => Some(if m.is_empty() { "{}".to_string() } else { format!("{{ {} alan }}", m.len()) }),
            Value::Array(a) => Some(format!("Array [{}]", a.len())),
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
