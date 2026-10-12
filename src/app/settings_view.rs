//! The settings page.

use super::*;
use gpui_kit::component::checkbox::Checkbox;

impl Humon {
    pub(crate) fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, hover, active_bg) = (
            theme.border,
            theme.muted_foreground,
            theme.list_hover,
            theme.list_active,
        );
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
        let chosen = self.settings.schema_sample();
        let mut samples = div().flex().gap_1();
        for &size in crate::schema::SAMPLE_SIZES {
            samples = samples.child(
                Button::new(SharedString::from(format!("sample-{size}")))
                    .label(crate::tab::status::thousands(size as u64))
                    .when(size == chosen, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.schema_sample = Some(size);
                        this.save_settings();
                        cx.notify();
                    })),
            );
        }
        div().flex_1().flex().justify_center().p_8().child(
            div()
                .w(px(460.))
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t!("settings.title").to_string()),
                        )
                        .child(
                            Button::new("settings-close")
                                .label(t!("settings.close"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_settings = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("settings.language").to_string()),
                )
                .child(languages)
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("settings.contribute").to_string()),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("settings.updates").to_string()),
                )
                .child(
                    Checkbox::new("check-updates")
                        .label(t!("settings.check_updates").to_string())
                        .checked(self.settings.check_updates())
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.settings.check_updates = Some(*checked);
                            this.save_settings();
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("settings.schema").to_string()),
                )
                .child(div().child(t!("settings.schema_sample").to_string()))
                .child(samples)
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("settings.schema_hint").to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("settings.version", version = crate::version::VERSION).to_string()),
                ),
        )
    }
}
