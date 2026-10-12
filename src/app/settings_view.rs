//! The settings page.

use super::*;

impl Humon {
    pub(crate) fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, hover, active_bg) = (theme.border, theme.muted_foreground, theme.list_hover, theme.list_active);
        let current = rust_i18n::locale().to_string();
        let mut languages = div().flex().flex_col().gap_1();
        for code in settings::languages() {
            let selected = code == current;
            let name = t!("_language_name", locale = &code).to_string();
            let pick = code.clone();
            languages = languages.child(
                div()
                    .id(SharedString::from(format!("lang-{code}")))
                    .flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(border)
                    .cursor_pointer()
                    .when(selected, |d| d.bg(active_bg))
                    .hover(|s| s.bg(hover))
                    .child(div().w(px(14.)).child(if selected { "✓" } else { "" }))
                    .child(div().flex_1().child(name))
                    .child(div().text_color(muted).child(code))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_language(&pick, window, cx);
                        cx.notify();
                    })),
            );
        }
        div()
            .flex_1()
            .flex()
            .justify_center()
            .p_8()
            .child(
                div()
                    .w(px(460.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(div().flex_1().text_xl().font_weight(FontWeight::SEMIBOLD).child(t!("settings.title").to_string()))
                            .child(Button::new("settings-close").label(t!("settings.close")).on_click(cx.listener(|this, _, _, cx| {
                                this.show_settings = false;
                                cx.notify();
                            }))),
                    )
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(t!("settings.language").to_string()))
                    .child(languages)
                    .child(div().text_xs().text_color(muted).child(t!("settings.contribute").to_string())),
            )
    }
}
