//! Root view: sidebar with saved connections and the db/collection tree, the
//! tab bar, and the active collection tab (see tab.rs).

use std::future::Future;
use std::sync::Arc;

use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mongodb::Client;

use crate::console::view::ConsoleTab;
use crate::history::History;
use crate::manage::{EditForm, SidebarEdit};
use crate::mongo::{self, CollKind, CollectionInfo, CollectionRef, DatabaseInfo, RT};
use crate::settings::{self, Settings};
use crate::store::{SavedConnection, Store};
use crate::tab::Tab;
use rust_i18n::t;

mod connection_form;
mod connections;
mod quick_open;
mod session;
mod settings_view;
mod sidebar;
mod tabs_bar;

pub struct DbNode {
    pub info: DatabaseInfo,
    pub open: bool,
    pub loading: bool,
    pub collections: Option<Vec<CollectionInfo>>,
}

pub struct ConnNode {
    pub saved: SavedConnection,
    pub client: Option<Client>,
    pub connecting: bool,
    pub open: bool,
    pub error: Option<String>,
    pub dbs: Vec<DbNode>,
}

pub struct Humon {
    pub conns: Vec<ConnNode>,
    pub tabs: Vec<Tab>,
    pub consoles: Vec<ConsoleTab>,
    pub active_tab: Option<usize>,
    next_tab_id: usize,
    adding: bool,
    form_name: Entity<InputState>,
    form_uri: Entity<InputState>,
    form_color: Option<String>,
    form_error: Option<String>,
    /// Saved id awaiting a second click to confirm deletion.
    confirm_delete: Option<String>,
    /// Saved id whose color palette is open.
    color_picker: Option<String>,
    search: Entity<InputState>,
    /// Lowercased collection search; empty shows the normal tree.
    query: String,
    pub load_error: Option<String>,
    pub history: History,
    store: Store,
    pub form: EditForm,
    show_settings: bool,
    /// The settings page is showing as the first-launch welcome.
    welcome: bool,
    /// Kept in memory and saved whole, so changing one setting keeps the others.
    pub settings: Settings,
    /// A newer release on GitHub, shown at the bottom of the sidebar.
    pub update: Option<crate::version::Release>,
    pub palette_open: bool,
    palette_input: Entity<InputState>,
    palette_selected: usize,
    focus: FocusHandle,
    /// Tabs from the last launch whose connection isn't open yet, with
    /// their place in the saved order.
    pending: Vec<(usize, crate::session::SavedTab)>,
    /// The saved order's active tab, made active again when it opens.
    pending_active: Option<usize>,
    /// The open tabs changed; the session file is rewritten on the next render.
    pub(crate) session_dirty: bool,
    /// Decoded once; drawn in the sidebar's header.
    pub(crate) logo: std::sync::Arc<gpui_kit::Image>,
    /// session.json couldn't be read; it is kept as it is for the user to look at.
    session_broken: bool,
    _subscriptions: Vec<Subscription>,
}

gpui_kit::actions!(humon, [QuickOpen]);

/// The app logo, embedded so the binary needs no files next to it.
fn logo() -> Arc<Image> {
    static LOGO: std::sync::LazyLock<Arc<Image>> = std::sync::LazyLock::new(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../assets/logo-256.png").to_vec(),
        ))
    });
    LOGO.clone()
}

/// Connection label colors, stored as `#rrggbb`.
const PALETTE: [&str; 8] = [
    "#e5484d", "#f76b15", "#ffc53d", "#30a46c", "#12a594", "#0090ff", "#8e4ec6", "#d6409f",
];

fn parse_color(hex: &str) -> Option<Hsla> {
    u32::from_str_radix(hex.strip_prefix('#')?, 16)
        .ok()
        .map(|v| rgb(v).into())
}

/// A collection to open, by saved connection.
pub struct CollectionAt {
    pub conn_id: String,
    pub db: String,
    pub coll: String,
    pub kind: CollKind,
}

/// After closing tab `closed`, the tab to activate among `open_ids` (in tab
/// order): the one to its right, else the one to its left.
pub fn next_active(open_ids: &[usize], closed: usize) -> Option<usize> {
    let rest: Vec<usize> = open_ids.iter().copied().filter(|&id| id != closed).collect();
    rest.iter()
        .copied()
        .find(|&id| id > closed)
        .or_else(|| rest.iter().copied().rev().find(|&id| id < closed))
}

/// Which of a database's collections the tree shows for a search; `None`
/// hides the database. An empty query or a matching database name shows all.
pub fn visible_collections<'a>(
    db_name: &str,
    cols: Option<&'a [CollectionInfo]>,
    query: &str,
) -> Option<Vec<&'a CollectionInfo>> {
    let cols = cols.unwrap_or(&[]);
    let q = query.to_lowercase();
    if q.is_empty() || db_name.to_lowercase().contains(&q) {
        return Some(cols.iter().collect());
    }
    let matching: Vec<_> = cols.iter().filter(|c| c.name.to_lowercase().contains(&q)).collect();
    (!matching.is_empty()).then_some(matching)
}

impl ConnNode {
    pub fn color(&self) -> Option<Hsla> {
        self.saved.color.as_deref().and_then(parse_color)
    }
}

pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64 / 1024.0;
    let mut unit = "KB";
    for u in ["MB", "GB", "TB"] {
        if v < 1024.0 {
            break;
        }
        v /= 1024.0;
        unit = u;
    }
    if v < 10.0 {
        format!("{v:.1} {unit}")
    } else {
        format!("{v:.0} {unit}")
    }
}

impl Humon {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let form_name = cx.new(|cx| InputState::new(window, cx).placeholder(t!("form.name")));
        let form_uri = cx.new(|cx| InputState::new(window, cx).placeholder(t!("form.uri")));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t!("sidebar.search")));
        let search_sub = cx.subscribe_in(&search, window, |this, _, ev: &InputEvent, window, cx| {
            if let InputEvent::Change = ev {
                this.on_search(window, cx);
            }
        });
        let (form, form_subs) = EditForm::new(window, cx);
        let palette_input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("palette.placeholder")));
        let palette_sub = cx.subscribe_in(
            &palette_input,
            window,
            |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    this.palette_selected = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.activate_palette(window, cx);
                    cx.notify();
                }
                _ => {}
            },
        );
        let settings = Settings::load_from(&Settings::default_path()).unwrap_or_default();
        settings.apply();
        if settings.check_updates() {
            Self::check_for_update(cx);
        }
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let store = Store::default();
        // An unreadable history file is reported and left alone, never overwritten.
        let (history, history_error) = match History::open(History::default_path()) {
            Ok(h) => (h, None),
            Err(e) => (History::default(), Some(e)),
        };
        let (saved, load_error) = match store.list() {
            Ok(list) => (list, None),
            Err(e) => (vec![], Some(e)),
        };
        // Passwords from before the Keychain move there now; failures are reported, never lost.
        let migrate_error = store.migrate_secrets().err();
        let load_error = load_error.or(history_error).or(migrate_error);
        // The tabs of the last launch reopen as their connections come up. An
        // unreadable session file is reported and left alone, never overwritten.
        let (session, session_error) = match crate::session::Session::load_from(&crate::session::Session::default_path())
        {
            Ok(session) => (session, None),
            Err(e) => (crate::session::Session::default(), Some(e)),
        };
        let session_broken = session_error.is_some();
        let load_error = load_error.or(session_error);
        // If the last launch closed while reopening tabs, one of them may be why:
        // start without them this time instead of failing the same way again.
        let (session, skipped) = if !settings.restore_session() {
            (crate::session::Session::default(), None)
        } else if crate::session::restore_crashed() {
            (
                crate::session::Session::default(),
                Some(t!("app.session_skipped").to_string()),
            )
        } else {
            (session, None)
        };
        let load_error = load_error.or(skipped);
        let quit_sub = cx.on_app_quit(|this: &mut Self, cx| {
            this.persist_session(cx);
            async {}
        });
        cx.defer_in(window, |this, window, cx| this.restore_session(window, cx));
        let adding = saved.is_empty();
        let welcome = settings.is_first_launch(!saved.is_empty());
        Self {
            conns: saved
                .into_iter()
                .map(|saved| ConnNode {
                    saved,
                    client: None,
                    connecting: false,
                    open: false,
                    error: None,
                    dbs: vec![],
                })
                .collect(),
            tabs: vec![],
            active_tab: None,
            next_tab_id: 1,
            consoles: Vec::new(),
            adding,
            form_name,
            form_uri,
            form_color: None,
            form_error: None,
            confirm_delete: None,
            color_picker: None,
            search,
            query: String::new(),
            load_error,
            history,
            store,
            form,
            show_settings: welcome,
            welcome,
            settings,
            update: None,
            palette_open: false,
            palette_input,
            palette_selected: 0,
            focus,
            pending: session.tabs.into_iter().enumerate().collect(),
            pending_active: session.active,
            session_dirty: false,
            logo: std::sync::Arc::new(gpui_kit::Image::from_bytes(
                gpui_kit::ImageFormat::Png,
                include_bytes!("../../assets/logo-64.png").to_vec(),
            )),
            session_broken,
            _subscriptions: [search_sub, palette_sub, quit_sub]
                .into_iter()
                .chain(form_subs)
                .collect(),
        }
    }

    /// Runs a driver future on tokio and applies the result back on the UI thread.
    pub fn run<T: Send + 'static>(
        &self,
        fut: impl Future<Output = T> + Send + 'static,
        window: &Window,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let handle = RT.spawn(fut);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(value) = handle.await else { return };
            this.update_in(cx, |this, window, cx| {
                apply(this, value, window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn conn_ix(&self, saved_id: &str) -> Option<usize> {
        self.conns.iter().position(|c| c.saved.id == saved_id)
    }

    pub fn close_tabs_where(&mut self, pred: impl Fn(&Tab) -> bool) {
        let ids: Vec<usize> = self.tabs.iter().filter(|t| pred(t)).map(|t| t.id).collect();
        for id in ids {
            self.close_tab(id);
        }
    }

    pub fn save_settings(&mut self) {
        self.settings.apply();
        if let Err(e) = self.settings.save_to(&Settings::default_path()) {
            self.load_error = Some(e);
        }
    }

    /// Asks GitHub once, in the background, whether a newer release exists.
    fn check_for_update(cx: &mut Context<Self>) {
        let task = RT.spawn_blocking(crate::version::fetch_latest);
        cx.spawn(async move |this, cx| {
            if let Ok(Some(release)) = task.await
                && crate::version::is_newer(crate::version::VERSION, &release.version)
            {
                this.update(cx, |this, cx| {
                    this.update = Some(release);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn set_language(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        rust_i18n::set_locale(code);
        self.settings.language = Some(code.to_string());
        self.save_settings();
        // Placeholders are held by the inputs, so they need re-setting; everything
        // else is read through t! on the next render.
        for (input, key) in [
            (&self.form_name, "form.name"),
            (&self.form_uri, "form.uri"),
            (&self.search, "sidebar.search"),
        ] {
            input.update(cx, |s, cx| s.set_placeholder(t!(key), window, cx));
        }
        self.relabel_tabs(window, cx);
    }

    /// Focuses the collection's tab or opens one; `new_tab` always opens another.
    fn open_collection(&mut self, at: &CollectionAt, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !new_tab
            && let Some(tab) = self
                .tabs
                .iter()
                .find(|t| t.conn_id == at.conn_id && t.target.db == at.db && t.target.name == at.coll)
        {
            self.active_tab = Some(tab.id);
            return;
        }
        let Some(ix) = self.conn_ix(&at.conn_id) else { return };
        let Some(client) = self.conns[ix].client.clone() else {
            return;
        };
        let target = CollectionRef::new(client, &at.db, &at.coll);
        let id = self.add_tab(&at.conn_id, target, at.kind, window, cx, |_| {});
        self.active_tab = Some(id);
        self.session_dirty = true;
        self.run_query(id, window, cx);
        self.load_schema(id, window, cx);
    }

    /// Adds a collection tab, set up by `prepare` before it is shown; returns its id.
    pub(crate) fn add_tab(
        &mut self,
        conn_id: &str,
        target: CollectionRef,
        kind: CollKind,
        window: &mut Window,
        cx: &mut Context<Self>,
        prepare: impl FnOnce(&mut Tab),
    ) -> usize {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let mut tab = Tab::new(id, conn_id, target, kind, window, cx);
        prepare(&mut tab);
        self.tabs.push(tab);
        id
    }

    /// Every open tab's id, collections and shells together, in tab order.
    pub fn tab_ids(&self) -> Vec<usize> {
        let mut ids: Vec<usize> = self
            .tabs
            .iter()
            .map(|t| t.id)
            .chain(self.consoles.iter().map(|c| c.id))
            .collect();
        ids.sort_unstable();
        ids
    }

    pub fn close_tab(&mut self, id: usize) {
        self.session_dirty = true;
        let ids = self.tab_ids();
        self.tabs.retain(|t| t.id != id);
        // Dropping a shell's engine handle stops and ends its thread.
        self.consoles.retain(|c| c.id != id);
        if self.active_tab == Some(id) {
            self.active_tab = next_active(&ids, id);
        }
    }

    /// Opens a shell tab on a connection, starting in `db`.
    pub fn open_console(
        &mut self,
        saved_id: &str,
        db: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let client = self
            .conns
            .iter()
            .find(|c| c.saved.id == saved_id)
            .and_then(|c| c.client.clone())?;
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let sampling = self.settings.sampling();
        let console = ConsoleTab::new(id, saved_id, client, db, sampling, window, cx);
        self.consoles.push(console);
        self.active_tab = Some(id);
        self.session_dirty = true;
        self.close_settings();
        Some(id)
    }
}

impl Humon {
    /// Leaves the settings page; the first time, that ends the welcome for good.
    pub(crate) fn close_settings(&mut self) {
        self.show_settings = false;
        if self.welcome {
            self.welcome = false;
            self.settings.welcomed = Some(true);
            self.save_settings();
        }
    }
}

impl Render for Humon {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.session_dirty {
            self.session_dirty = false;
            self.persist_session(cx);
        }
        let theme = cx.theme();
        let (bg, fg, muted) = (theme.background, theme.foreground, theme.muted_foreground);
        let console = self
            .active_tab
            .and_then(|id| self.consoles.iter().position(|c| c.id == id));
        let content = match self.active_tab.and_then(|id| self.tabs.iter().position(|t| t.id == id)) {
            _ if self.show_settings => self.render_settings(cx).into_any_element(),
            _ if console.is_some() => self.render_console(console.unwrap_or_default(), cx).into_any_element(),
            Some(ix) => self.render_tab(ix, window, cx).into_any_element(),
            None => div()
                .flex_1()
                .flex()
                .flex_col()
                .gap_4()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(img(logo()).size(px(112.)))
                .child(
                    if self.conns.is_empty() {
                        t!("app.empty_first")
                    } else {
                        t!("app.empty_pick")
                    }
                    .to_string(),
                )
                .into_any_element(),
        };
        div()
            .size_full()
            .relative()
            .flex()
            .bg(bg)
            .text_color(fg)
            .text_size(px(crate::ui::TEXT))
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &QuickOpen, window, cx| {
                this.open_palette(window, cx);
                cx.notify();
            }))
            .child(self.render_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(self.render_quick_open_bar(cx))
                    .when(!self.tab_ids().is_empty(), |d| d.child(self.render_tabs(cx)))
                    .child(content),
            )
            // Painted last so it sits above everything else.
            .when(self.palette_open, |d| d.child(self.render_quick_open(cx)))
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's `test` macro, which shadows `#[test]`.
    use super::{format_size, next_active, parse_color, visible_collections};
    use crate::mongo::{CollKind, CollectionInfo};

    fn cols(names: &[&str]) -> Vec<CollectionInfo> {
        names
            .iter()
            .map(|n| CollectionInfo {
                name: n.to_string(),
                kind: CollKind::Collection,
            })
            .collect()
    }

    fn names(v: Option<Vec<&CollectionInfo>>) -> Option<Vec<&str>> {
        v.map(|v| v.into_iter().map(|c| c.name.as_str()).collect())
    }

    #[test]
    fn search_filters_collections() {
        let c = cols(&["users", "orders", "UserEvents"]);
        assert_eq!(
            names(visible_collections("shop", Some(&c), "")),
            Some(vec!["users", "orders", "UserEvents"])
        );
        assert_eq!(
            names(visible_collections("shop", Some(&c), "user")),
            Some(vec!["users", "UserEvents"]),
            "case-insensitive"
        );
        assert_eq!(
            names(visible_collections("shop", Some(&c), "sho")),
            Some(vec!["users", "orders", "UserEvents"]),
            "db name match shows all"
        );
        assert_eq!(names(visible_collections("shop", Some(&c), "zzz")), None);
        // Not loaded yet: visible while searching only if the db name matches.
        assert_eq!(names(visible_collections("shop", None, "sho")), Some(vec![]));
        assert_eq!(names(visible_collections("shop", None, "users")), None);
    }

    #[test]
    fn closing_a_tab_activates_its_neighbour() {
        assert_eq!(next_active(&[1, 2, 3], 2), Some(3), "the one to the right");
        assert_eq!(next_active(&[1, 2, 3], 3), Some(2), "or the left when it was last");
        assert_eq!(next_active(&[4], 4), None);
        assert_eq!(
            next_active(&[1, 5, 9], 7),
            Some(9),
            "an id that is not open still picks a neighbour"
        );
    }

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(20 * 1024 * 1024), "20 MB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn colors_parse_from_hex() {
        assert!(parse_color("#e5484d").is_some());
        assert!(parse_color("e5484d").is_none(), "needs the #");
        assert!(parse_color("#zzzzzz").is_none());
    }
}
