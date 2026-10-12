//! Quick open (⌘K): the search box above the tabs and the list it opens.

use super::*;
use crate::palette::{self, Item, Target};
use gpui_kit::component::input::{Escape, MoveDown, MoveUp};

/// Most rows the list shows; typing narrows the rest.
const MAX_ROWS: usize = 12;

impl Humon {
    /// Every collection and every database's shell on the connected connections.
    pub(crate) fn palette_items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        for conn in self.conns.iter().filter(|c| c.client.is_some()) {
            for db in &conn.dbs {
                let item = |target| Item {
                    conn_id: conn.saved.id.clone(),
                    conn_name: conn.saved.name.clone(),
                    db: db.info.name.clone(),
                    target,
                };
                for coll in db.collections.iter().flatten() {
                    items.push(item(Target::Collection(coll.name.clone())));
                }
                items.push(item(Target::Shell));
            }
        }
        items
    }

    pub(crate) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = true;
        self.palette_selected = 0;
        self.palette_input.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.focus(window, cx);
        });
        // Collections of databases never expanded aren't known yet.
        let missing: Vec<(String, String)> = self
            .conns
            .iter()
            .filter(|c| c.client.is_some())
            .flat_map(|c| {
                c.dbs
                    .iter()
                    .filter(|d| d.collections.is_none())
                    .map(|d| (c.saved.id.clone(), d.info.name.clone()))
            })
            .collect();
        for (id, db) in missing {
            self.load_collections(&id, &db, window, cx);
        }
    }

    fn palette_matches(&self, cx: &App) -> (Vec<Item>, Vec<usize>) {
        let items = self.palette_items();
        let ranked = palette::rank(&items, &self.palette_input.read(cx).value());
        (items, ranked)
    }

    pub(crate) fn activate_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (items, ranked) = self.palette_matches(cx);
        let Some(item) = ranked.get(self.palette_selected).map(|&i| items[i].clone()) else {
            return;
        };
        self.palette_open = false;
        match item.target {
            Target::Shell => {
                self.open_console(&item.conn_id, &item.db, window, cx);
            }
            Target::Collection(coll) => {
                let kind = self
                    .conns
                    .iter()
                    .find(|c| c.saved.id == item.conn_id)
                    .and_then(|c| c.dbs.iter().find(|d| d.info.name == item.db))
                    .and_then(|d| d.collections.iter().flatten().find(|c| c.name == coll))
                    .map_or(CollKind::Collection, |c| c.kind);
                self.show_settings = false;
                let at = CollectionAt {
                    conn_id: item.conn_id,
                    db: item.db,
                    coll,
                    kind,
                };
                self.open_collection(&at, false, window, cx);
            }
        }
    }

    /// Moves the selection; wraps around at either end.
    fn palette_move(&mut self, down: bool, cx: &mut Context<Self>) {
        let count = self.palette_matches(cx).1.len().min(MAX_ROWS);
        if count > 0 {
            self.palette_selected = if down {
                (self.palette_selected + 1) % count
            } else {
                (self.palette_selected + count - 1) % count
            };
        }
        cx.notify();
    }

    /// The search box above the tabs; clicking it opens quick open.
    pub(crate) fn render_quick_open_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, panel, bg, hover) = (
            theme.border,
            theme.muted_foreground,
            theme.sidebar,
            theme.background,
            theme.list_hover,
        );
        div()
            .flex()
            .flex_none()
            .justify_center()
            .items_center()
            .h(px(38.))
            .border_b_1()
            .border_color(border)
            .bg(panel)
            .child(
                div()
                    .id("quick-open")
                    .flex()
                    .items_center()
                    .gap_2()
                    .w(px(440.))
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(border)
                    .bg(bg)
                    .text_color(muted)
                    .cursor_pointer()
                    .hover(|s| s.bg(hover))
                    .child("⌕")
                    .child(div().flex_1().child(t!("palette.placeholder").to_string()))
                    .child(div().text_xs().child("⌘K"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_palette(window, cx);
                        cx.notify();
                    })),
            )
    }

    /// The quick open list over the window; clicking outside closes it.
    pub(crate) fn render_quick_open(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, bg, hover, active) = (
            theme.border,
            theme.muted_foreground,
            theme.background,
            theme.list_hover,
            theme.list_active,
        );
        let (items, ranked) = self.palette_matches(cx);
        let selected = self.palette_selected.min(ranked.len().saturating_sub(1));
        let mut rows = div().flex().flex_col().py_1();
        if ranked.is_empty() {
            rows = rows.child(
                div()
                    .px_3()
                    .py_2()
                    .text_color(muted)
                    .child(t!("palette.empty").to_string()),
            );
        }
        for (row, &ix) in ranked.iter().take(MAX_ROWS).enumerate() {
            let item = &items[ix];
            let color = self
                .conns
                .iter()
                .find(|c| c.saved.id == item.conn_id)
                .and_then(ConnNode::color);
            let icon = if item.target == Target::Shell { ">_" } else { "▤" };
            rows = rows.child(
                div()
                    .id(("palette-row", row))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .when(row == selected, |d| d.bg(active))
                    .hover(|s| s.bg(hover))
                    .child(div().w(px(18.)).text_color(muted).child(icon))
                    .child(div().flex_1().min_w_0().truncate().child(item.label()))
                    .when_some(color, |d, c| d.child(div().size(px(7.)).rounded_full().bg(c)))
                    .child(div().text_xs().text_color(muted).child(item.conn_name.clone()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.palette_selected = row;
                        this.activate_palette(window, cx);
                        cx.notify();
                    })),
            );
        }
        div()
            .id("palette-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .pt(px(56.))
            .bg(gpui_kit::hsla(0., 0., 0., 0.25))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.palette_open = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .id("palette")
                    .w(px(580.))
                    .max_h(px(460.))
                    .flex()
                    .flex_col()
                    .bg(bg)
                    .border_1()
                    .border_color(border)
                    .rounded_lg()
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    // The input binds up/down/escape to its own actions, so take
                    // them in the capture phase, before it sees them.
                    .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                        cx.stop_propagation();
                        this.palette_move(false, cx);
                    }))
                    .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                        cx.stop_propagation();
                        this.palette_move(true, cx);
                    }))
                    .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                        cx.stop_propagation();
                        this.palette_open = false;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .p_2()
                            .border_b_1()
                            .border_color(border)
                            .child(Input::new(&self.palette_input)),
                    )
                    .child(rows),
            )
    }
}
