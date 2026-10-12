//! Creating, renaming and dropping databases and collections from the
//! sidebar. Each action opens a small inline form under its row; drops
//! require typing the name, as Compass does.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;

use crate::app::Humon;
use crate::history;
use crate::mongo::{self, CollectionRef};

#[derive(Clone, PartialEq, Eq)]
pub enum SidebarEdit {
    NewDb { conn: String },
    NewColl { conn: String, db: String },
    RenameColl { conn: String, db: String, coll: String },
    DropColl { conn: String, db: String, coll: String },
    DropDb { conn: String, db: String },
}

impl SidebarEdit {
    fn conn(&self) -> &str {
        match self {
            Self::NewDb { conn }
            | Self::NewColl { conn, .. }
            | Self::RenameColl { conn, .. }
            | Self::DropColl { conn, .. }
            | Self::DropDb { conn, .. } => conn,
        }
    }

    /// The name that must be typed to confirm a drop.
    fn confirm_name(&self) -> Option<&str> {
        match self {
            Self::DropColl { coll, .. } => Some(coll),
            Self::DropDb { db, .. } => Some(db),
            _ => None,
        }
    }
}

/// Whether the form's input allows submitting.
#[derive(Debug, PartialEq, Eq)]
pub enum Validation {
    Ok,
    /// A drop whose typed name doesn't match yet; the button stays disabled.
    NotReady,
    NameRequired,
}

pub fn validate(edit: &SidebarEdit, name: &str, second: &str) -> Validation {
    let (name, second) = (name.trim(), second.trim());
    if let Some(expected) = edit.confirm_name() {
        return if name == expected {
            Validation::Ok
        } else {
            Validation::NotReady
        };
    }
    if name.is_empty() || (matches!(edit, SidebarEdit::NewDb { .. }) && second.is_empty()) {
        Validation::NameRequired
    } else {
        Validation::Ok
    }
}

/// State of the inline form.
pub struct EditForm {
    pub edit: Option<SidebarEdit>,
    name: Entity<InputState>,
    second: Entity<InputState>,
    error: Option<String>,
    busy: bool,
}

impl EditForm {
    pub fn new(window: &mut Window, cx: &mut Context<Humon>) -> (Self, Vec<Subscription>) {
        let name = cx.new(|cx| InputState::new(window, cx));
        let second = cx.new(|cx| InputState::new(window, cx));
        let subs = [&name, &second]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(input, window, |this: &mut Humon, _, ev: &InputEvent, window, cx| {
                    match ev {
                        InputEvent::PressEnter { .. } => this.submit_edit(window, cx),
                        // The drop button enables once the typed name matches.
                        InputEvent::Change => cx.notify(),
                        _ => {}
                    }
                })
            })
            .collect();
        (
            Self {
                edit: None,
                name,
                second,
                error: None,
                busy: false,
            },
            subs,
        )
    }

    pub fn is(&self, edit: &SidebarEdit) -> bool {
        self.edit.as_ref() == Some(edit)
    }
}

impl Humon {
    pub fn start_sidebar_edit(&mut self, edit: SidebarEdit, window: &mut Window, cx: &mut Context<Self>) {
        let (value, placeholder, second_placeholder) = match &edit {
            SidebarEdit::NewDb { .. } => (String::new(), t!("manage.db_name"), t!("manage.first_coll")),
            SidebarEdit::NewColl { .. } => (String::new(), t!("manage.coll_name"), "".into()),
            SidebarEdit::RenameColl { coll, .. } => (coll.clone(), t!("manage.new_name"), "".into()),
            SidebarEdit::DropColl { coll: name, .. } | SidebarEdit::DropDb { db: name, .. } => {
                (String::new(), name.clone().into(), "".into())
            }
        };
        let form = &mut self.form;
        form.error = None;
        form.busy = false;
        form.edit = Some(edit);
        form.name.update(cx, |s, cx| {
            s.set_value(value, window, cx);
            s.set_placeholder(placeholder, window, cx);
            s.focus(window, cx);
        });
        form.second.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.set_placeholder(second_placeholder, window, cx);
        });
    }

    pub fn cancel_edit(&mut self) {
        self.form.edit = None;
    }

    fn submit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.form.edit.clone() else { return };
        if self.form.busy {
            return;
        }
        let name = self.form.name.read(cx).value().trim().to_string();
        let second = self.form.second.read(cx).value().trim().to_string();
        match validate(&edit, &name, &second) {
            Validation::Ok => {}
            Validation::NotReady => return,
            Validation::NameRequired => {
                self.form.error = Some(t!("manage.name_required").into());
                return;
            }
        }
        let Some(client) = self
            .conns
            .iter()
            .find(|c| c.saved.id == edit.conn())
            .and_then(|c| c.client.clone())
        else {
            return;
        };
        self.form.busy = true;
        self.form.error = None;
        let fut = {
            let edit = edit.clone();
            async move {
                match edit {
                    // The new database's name is `name`, its first collection `second`.
                    SidebarEdit::NewDb { .. } => CollectionRef::new(client, &name, &second).create().await,
                    SidebarEdit::NewColl { db, .. } => CollectionRef::new(client, &db, &name).create().await,
                    SidebarEdit::RenameColl { db, coll, .. } => {
                        CollectionRef::new(client, &db, &coll).rename(name).await
                    }
                    SidebarEdit::DropColl { db, coll, .. } => CollectionRef::new(client, &db, &coll).drop().await,
                    SidebarEdit::DropDb { db, .. } => mongo::drop_database(client, db).await,
                }
            }
        };
        let new_name = self.form.name.read(cx).value().trim().to_string();
        self.run(fut, window, cx, move |this, res, window, cx| {
            this.form.busy = false;
            if let Err(e) = res {
                this.form.error = Some(e);
                return;
            }
            this.form.edit = None;
            this.after_edit(edit, &new_name, window, cx);
        });
    }

    /// Brings the tree, open tabs and query history in line with a finished edit.
    fn after_edit(&mut self, edit: SidebarEdit, new_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        match edit {
            SidebarEdit::NewDb { conn } => self.refresh_dbs(&conn, window, cx),
            SidebarEdit::NewColl { conn, db } => self.reload_collections(&conn, &db, window, cx),
            SidebarEdit::RenameColl { conn, db, coll } => {
                for tab in self
                    .tabs
                    .iter_mut()
                    .filter(|t| t.conn_id == conn && t.target.db == db && t.target.name == coll)
                {
                    tab.target.name = new_name.to_string();
                }
                self.history
                    .rename(&history::key(&conn, &db, &coll), &history::key(&conn, &db, new_name));
                self.persist_history();
                self.reload_collections(&conn, &db, window, cx);
            }
            SidebarEdit::DropColl { conn, db, coll } => {
                self.close_tabs_where(|t| t.conn_id == conn && t.target.db == db && t.target.name == coll);
                self.history.forget(&history::key(&conn, &db, &coll));
                self.persist_history();
                self.reload_collections(&conn, &db, window, cx);
            }
            SidebarEdit::DropDb { conn, db } => {
                self.close_tabs_where(|t| t.conn_id == conn && t.target.db == db);
                self.history.forget(&format!("{conn}/{db}"));
                self.persist_history();
                self.refresh_dbs(&conn, window, cx);
            }
        }
    }

    pub fn persist_history(&mut self) {
        if let Err(e) = self.history.save() {
            self.load_error = Some(e);
        }
    }

    /// The inline form for `edit`, rendered under the row it belongs to.
    pub fn render_edit_form(&self, indent: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger, border) = (theme.muted_foreground, theme.danger, theme.border);
        let form = &self.form;
        let edit = form.edit.clone();
        let (title, button) = match &edit {
            Some(SidebarEdit::NewDb { .. }) => (t!("manage.new_db"), t!("manage.create")),
            Some(SidebarEdit::NewColl { .. }) => (t!("manage.new_coll"), t!("manage.create")),
            Some(SidebarEdit::RenameColl { .. }) => (t!("manage.rename"), t!("manage.rename_button")),
            Some(SidebarEdit::DropColl { .. }) => (t!("manage.drop_coll"), t!("manage.drop_button")),
            Some(SidebarEdit::DropDb { .. }) => (t!("manage.drop_db"), t!("manage.drop_button")),
            None => return div(),
        };
        let confirm = edit.as_ref().and_then(|e| e.confirm_name().map(str::to_string));
        let ready = edit
            .as_ref()
            .is_some_and(|e| validate(e, &form.name.read(cx).value(), "") != Validation::NotReady);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .pl(indent)
            .pr_3()
            .py_2()
            .border_y_1()
            .border_color(border)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.to_string()),
            )
            .when_some(confirm.clone(), |d, name| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(danger)
                        .child(t!("manage.type_to_confirm", name = name).to_string()),
                )
            })
            .when(matches!(edit, Some(SidebarEdit::NewDb { .. })), |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("manage.new_db_hint").to_string()),
                )
            })
            .child(Input::new(&form.name))
            .when(matches!(edit, Some(SidebarEdit::NewDb { .. })), |d| {
                d.child(Input::new(&form.second))
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("edit-cancel")
                            .label(t!("manage.cancel"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_edit();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("edit-submit")
                            .label(button)
                            .map(|b| if confirm.is_some() { b.danger() } else { b.primary() })
                            .disabled(!ready || form.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.submit_edit(window, cx);
                                cx.notify();
                            })),
                    ),
            )
            .when_some(form.error.clone(), |d, e| {
                d.child(div().text_xs().text_color(danger).child(e))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{SidebarEdit, Validation, validate};

    fn drop_coll() -> SidebarEdit {
        SidebarEdit::DropColl {
            conn: "c".into(),
            db: "shop".into(),
            coll: "users".into(),
        }
    }

    #[test]
    fn drops_need_the_exact_name() {
        assert_eq!(validate(&drop_coll(), "", ""), Validation::NotReady);
        assert_eq!(validate(&drop_coll(), "user", ""), Validation::NotReady);
        assert_eq!(
            validate(&drop_coll(), "Users", ""),
            Validation::NotReady,
            "case matters"
        );
        assert_eq!(
            validate(&drop_coll(), "  users ", ""),
            Validation::Ok,
            "surrounding spaces are trimmed"
        );
        let drop_db = SidebarEdit::DropDb {
            conn: "c".into(),
            db: "shop".into(),
        };
        assert_eq!(validate(&drop_db, "shop", ""), Validation::Ok);
    }

    #[test]
    fn names_are_required() {
        let new_coll = SidebarEdit::NewColl {
            conn: "c".into(),
            db: "shop".into(),
        };
        assert_eq!(validate(&new_coll, "  ", ""), Validation::NameRequired);
        assert_eq!(validate(&new_coll, "events", ""), Validation::Ok);
        let new_db = SidebarEdit::NewDb { conn: "c".into() };
        assert_eq!(
            validate(&new_db, "logs", ""),
            Validation::NameRequired,
            "a database needs its first collection"
        );
        assert_eq!(validate(&new_db, "logs", "events"), Validation::Ok);
    }
}
