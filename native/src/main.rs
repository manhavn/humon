//! humon on gpui: a lightweight MongoDB client.

#[path = "../../src-tauri/src/query.rs"]
mod query;
mod app;
mod highlight;
mod mongo;
mod shell;
mod smart;
mod store;
mod tab;

use gpui_kit::component::Theme;
use gpui_kit::*;

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
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
