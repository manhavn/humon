//! The tab bar, with each tab marked in its connection's color.

use super::*;

impl Humon {
    pub(crate) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, hover, bg, panel, border, accent, fg) = (
            theme.muted_foreground,
            theme.list_hover,
            theme.background,
            theme.sidebar,
            theme.border,
            theme.primary,
            theme.foreground,
        );
        let mut bar = div()
            .id("tabs")
            .flex()
            .flex_none()
            .overflow_x_scroll()
            .bg(panel)
            .border_b_1()
            .border_color(border);
        // (id, connection, title, subtitle) for collections and shells alike.
        let mut labels: Vec<(usize, &str, String, String)> = self
            .tabs
            .iter()
            .map(|t| (t.id, t.conn_id.as_str(), t.target.name.clone(), t.target.db.clone()))
            .chain(self.consoles.iter().map(|c| {
                (
                    c.id,
                    c.conn_id.as_str(),
                    format!(">_ {}", t!("console.title")),
                    c.db.clone(),
                )
            }))
            .collect();
        labels.sort_by_key(|l| l.0);
        for (id, conn_id, title, subtitle) in labels {
            let active = self.active_tab == Some(id);
            let conn = self.conns.iter().find(|c| c.saved.id == conn_id);
            // The connection's color marks all its tabs; without one only the active tab is marked.
            let strip = conn.and_then(ConnNode::color).or(active.then_some(accent));
            let conn_name = conn.map(|c| c.saved.name.clone()).unwrap_or_default();
            let group: SharedString = format!("tab-{id}").into();
            bar = bar.child(
                div()
                    .id(("tab", id))
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .gap_2()
                    .max_w(px(240.))
                    .pl_3()
                    .pr_1()
                    .py_1()
                    .relative()
                    .border_r_1()
                    .border_color(border)
                    .cursor_pointer()
                    .text_color(if active { fg } else { muted })
                    .when(active, |d| d.bg(bg))
                    .when(!active, |d| d.hover(|s| s.bg(hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active_tab = Some(id);
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, _, cx| {
                            this.close_tab(id);
                            cx.notify();
                        }),
                    )
                    .when_some(strip, |d, color| {
                        d.child(div().absolute().top_0().left_0().right_0().h(px(2.)).bg(color))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(title.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(format!("{conn_name} · {subtitle}")),
                            ),
                    )
                    .child(
                        div()
                            .id(("close", id))
                            .px_1()
                            .rounded_sm()
                            .text_color(muted)
                            .when(!active, |d| d.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.)))
                            .hover(|s| s.bg(hover))
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(id);
                                cx.notify();
                            })),
                    ),
            );
        }
        bar
    }
}
