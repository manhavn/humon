//! The Schema view: every field of the collection, nested, with how often it
//! appears and the share of each type.

use super::*;

impl Humon {
    pub(crate) fn render_schema(&self, tab: &Tab, syntax: &Arc<HighlightTheme>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, accent, hover, success) = (
            theme.border,
            theme.muted_foreground,
            theme.primary,
            theme.list_hover,
            theme.success,
        );
        let key_color = syntax
            .style("property")
            .and_then(|s| s.color)
            .unwrap_or(theme.foreground);
        let id = tab.id;
        let Some(schema) = tab.schema.borrow().clone() else {
            return div()
                .p_3()
                .text_color(muted)
                .child(t!("schema.loading").to_string())
                .into_any_element();
        };
        let summary = div()
            .flex()
            .items_center()
            .gap_2()
            .pb_2()
            .text_xs()
            .text_color(muted)
            .child(t!("schema.sampled", n = thousands(schema.sampled)).to_string())
            .child("·")
            .child(if schema.complete {
                div().text_color(success).child(t!("schema.complete").to_string())
            } else {
                div().child(t!("schema.partial").to_string())
            });
        let chips = |types: &[(String, u8)]| {
            div().flex().flex_wrap().gap_1().children(types.iter().map(|(t, pct)| {
                div()
                    .px_1()
                    .border_1()
                    .border_color(border)
                    .rounded_sm()
                    .child(format!("{t} {pct}%"))
            }))
        };
        let mut rows = div().flex().flex_col();
        for (i, row) in schema.overview().into_iter().enumerate() {
            let path = row.path.clone();
            let share = row.presence.map_or("<1%".to_string(), |p| format!("{p}%"));
            let width = row.presence.unwrap_or(1) as f32 * 0.8;
            rows = rows.child(
                div()
                    .id(SharedString::from(format!("schema-{id}-{i}")))
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_1()
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                        if let Some(tab) = this.tab_mut(id) {
                            tab.notice = Some(t!("tree.copied").into());
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(240.))
                            .flex_none()
                            .pl(px(row.depth as f32 * 16.))
                            .truncate()
                            .text_color(key_color)
                            .child(row.name),
                    )
                    .child(
                        div()
                            .w(px(80.))
                            .h(px(6.))
                            .flex_none()
                            .rounded_sm()
                            .bg(border)
                            .child(div().h_full().w(px(width)).rounded_sm().bg(accent)),
                    )
                    .child(div().w(px(36.)).flex_none().text_color(muted).child(share))
                    .child(chips(&row.types))
                    .when(!row.items.is_empty(), |d| {
                        d.child(div().text_color(muted).child("["))
                            .child(chips(&row.items))
                            .child(div().text_color(muted).child("]"))
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .font_family(theme.mono_font_family.clone())
            .text_xs()
            .child(summary)
            .child(rows)
            .into_any_element()
    }
}
