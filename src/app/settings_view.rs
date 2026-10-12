//! The settings page, which is also the first-launch welcome.

use super::*;
use gpui_kit::component::checkbox::Checkbox;

/// A titled card holding one group of settings.
fn section(title: impl Into<SharedString>, border: Hsla, panel: Hsla) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(border)
        .bg(panel)
        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title.into()))
}

fn hint(text: impl Into<SharedString>, muted: Hsla) -> Div {
    div().text_xs().text_color(muted).child(text.into())
}

impl Humon {
    pub(crate) fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted, hover, active_bg, panel, accent) = (
            theme.border,
            theme.muted_foreground,
            theme.list_hover,
            theme.list_active,
            theme.sidebar,
            theme.primary,
        );

        let current = rust_i18n::locale().to_string();
        let mut languages = div().flex().flex_wrap().gap_2();
        for code in settings::languages() {
            let selected = code == current;
            let name = t!("_language_name", locale = &code).to_string();
            let pick = code.clone();
            languages = languages.child(
                div()
                    .id(SharedString::from(format!("lang-{code}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w(px(150.))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(if selected { accent } else { border })
                    .cursor_pointer()
                    .when(selected, |d| d.bg(active_bg))
                    .hover(|s| s.bg(hover))
                    .child(div().flex_1().child(name))
                    .child(div().text_xs().text_color(muted).child(code))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_language(&pick, window, cx);
                        cx.notify();
                    })),
            );
        }

        let chosen = self.settings.schema_sample();
        let mut samples = div().flex().flex_wrap().gap_1();
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

        let header = div()
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
                        this.close_settings();
                        cx.notify();
                    })),
            );

        let welcome = div()
            .flex()
            .flex_col()
            .gap_1()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(accent)
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("settings.welcome_title").to_string()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(t!("settings.welcome").to_string()),
            );

        let language = section(t!("settings.language"), border, panel)
            .child(languages)
            .child(hint(t!("settings.contribute"), muted));

        let suggestions = section(t!("settings.schema"), border, panel)
            .child(div().text_sm().child(t!("settings.schema_sample").to_string()))
            .child(samples)
            .child(hint(t!("settings.schema_hint"), muted));

        let updates = section(t!("settings.updates"), border, panel).child(
            Checkbox::new("check-updates")
                .label(t!("settings.check_updates").to_string())
                .checked(self.settings.check_updates())
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.settings.check_updates = Some(*checked);
                    this.save_settings();
                    cx.notify();
                })),
        );

        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(hint(t!("settings.version", version = crate::version::VERSION), muted))
            .child(
                div()
                    .id("settings-repo")
                    .text_xs()
                    .text_color(accent)
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .child(t!("settings.github").to_string())
                    .on_click(|_, _, cx| cx.open_url(crate::version::REPO_URL)),
            )
            .child(div().flex_1())
            .when(self.welcome, |d| {
                d.child(
                    Button::new("welcome-continue")
                        .primary()
                        .label(t!("settings.continue"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_settings();
                            cx.notify();
                        })),
                )
            });

        div()
            .id("settings-page")
            .flex_1()
            .size_full()
            .overflow_y_scroll()
            .child(
                div().flex().justify_center().px_6().py_8().child(
                    div()
                        .w_full()
                        .max_w(px(560.))
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(header)
                        .when(self.welcome, |d| d.child(welcome))
                        .child(language)
                        .child(suggestions)
                        .child(updates)
                        .child(footer),
                ),
            )
    }
}
