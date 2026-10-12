//! humon on gpui: a lightweight MongoDB client.

mod app;
mod console;
mod files;
mod highlight;
mod history;
mod manage;
mod mongo;
mod palette;
mod query;
mod settings;
mod shell;
mod smart;
mod store;
mod tab;

use gpui_kit::component::Theme;
use gpui_kit::*;

// Every file in locales/ becomes a selectable language; see locales/en.yml.
rust_i18n::i18n!("locales", fallback = "en");

fn main() {
    rust_i18n::set_locale(&settings::initial_language());
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-k", app::QuickOpen, None),
            KeyBinding::new("alt-up", console::view::HistoryPrev, Some(console::view::KEY_CONTEXT)),
            KeyBinding::new("alt-down", console::view::HistoryNext, Some(console::view::KEY_CONTEXT)),
        ]);
        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions { title: Some("humon".into()), ..Default::default() }),
                ..Default::default()
            },
            cx,
            |window, cx| {
                Theme::sync_system_appearance(Some(window), cx);
                cx.new(|cx| app::Humon::new(window, cx))
            },
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
