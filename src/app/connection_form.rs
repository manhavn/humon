//! The new-connection form and the color palette.

use super::*;
use gpui_kit::component::Sizable;

impl Humon {
    /// A row of color swatches plus "none"; `on_pick` receives the chosen color.
    pub(crate) fn render_palette(
        &self,
        key: &str,
        selected: Option<&str>,
        cx: &mut Context<Self>,
        on_pick: impl Fn(&mut Self, Option<String>) + Clone + 'static,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (border, fg, muted) = (theme.border, theme.foreground, theme.muted_foreground);
        let mut row = div().flex().items_center().gap_1();
        for hex in PALETTE {
            let on_pick = on_pick.clone();
            let chosen = selected == Some(hex);
            row = row.child(
                div()
                    .id(SharedString::from(format!("swatch-{key}-{hex}")))
                    .size(px(16.))
                    .rounded_full()
                    .bg(parse_color(hex).unwrap_or(border))
                    .border_2()
                    .border_color(if chosen { fg } else { gpui_kit::transparent_black() })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        on_pick(this, Some(hex.to_string()));
                        cx.notify();
                    })),
            );
        }
        row.child(
            div()
                .id(SharedString::from(format!("swatch-{key}-none")))
                .px_1()
                .text_xs()
                .text_color(muted)
                .cursor_pointer()
                .child(t!("form.no_color").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    on_pick(this, None);
                    cx.notify();
                })),
        )
    }

    pub(crate) fn render_form(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, danger, muted) = (theme.border, theme.danger, theme.muted_foreground);
        let palette = self.render_palette("form", self.form_color.as_deref(), cx, |this, color| {
            this.form_color = color
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(border)
            .child(Input::new(&self.form_name))
            .child(Input::new(&self.form_uri))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_xs().text_color(muted).child(t!("form.color").to_string()))
                    .child(palette),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .when(!self.conns.is_empty(), |d| {
                        d.child(
                            Button::new("form-cancel")
                                .small()
                                .label(t!("form.cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.adding = false;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Button::new("form-save")
                            .small()
                            .primary()
                            .label(t!("form.save"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_connection(window, cx);
                                cx.notify();
                            })),
                    ),
            )
            .when_some(self.form_error.clone(), |d, e| {
                d.child(div().text_xs().text_color(danger).child(e))
            })
    }
}
