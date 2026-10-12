//! humon on gpui: a lightweight MongoDB client.

mod app;
mod complete;
mod console;
mod files;
mod highlight;
mod history;
mod manage;
mod mongo;
mod palette;
mod query;
mod schema;
mod settings;
mod shell;
mod smart;
mod store;
mod tab;
mod version;

use gpui_kit::component::Theme;
use gpui_kit::*;

// Every file in locales/ becomes a selectable language; see locales/en.yml.
rust_i18n::i18n!("locales", fallback = "en");

fn main() {
    // Used by the release workflow, so notes come from the same tested code.
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, version] = &args[..]
        && flag == "--release-notes"
    {
        match version::release_notes(include_str!("../CHANGELOG.md"), version) {
            Some(notes) => println!("{notes}"),
            None => std::process::exit(1),
        }
        return;
    }
    let saved = settings::Settings::load_from(&settings::Settings::default_path()).unwrap_or_default();
    rust_i18n::set_locale(&settings::initial_language(&saved));
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
                titlebar: Some(TitlebarOptions {
                    title: Some("humon".into()),
                    ..Default::default()
                }),
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
