//! The shell tab, laid out like Studio 3T's IntelliShell: a multi-line
//! editor on top, a read-only results pane below that can be selected,
//! copied and searched like any editor.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Editor, EditorState, InputEvent, Position};
use gpui_kit::component::{ActiveTheme, Disableable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mongodb::Client;
use rust_i18n::t;

use crate::app::Humon;
use crate::console::engine::EngineHandle;
use crate::console::lines::{Entry, Line, LineKind, lines, transcript};
use crate::console::recall::Recall;
use crate::highlight::shell_editor;
use crate::smart::Smart;

gpui_kit::actions!(humon_shell, [HistoryPrev, HistoryNext]);

/// The key context for the shell's own shortcuts (⌥↑/⌥↓).
pub const KEY_CONTEXT: &str = "HumonShell";

/// Commands the history menu lists.
const HISTORY_ROWS: usize = 20;

pub struct ConsoleTab {
    pub id: usize,
    pub conn_id: String,
    /// The database `db` points at, shown in the toolbar.
    pub db: String,
    handle: Option<EngineHandle>,
    editor: Entity<EditorState>,
    results: Entity<EditorState>,
    smart: Smart,
    recall: Recall,
    entries: Vec<Entry>,
    running: bool,
    history_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl ConsoleTab {
    pub fn new(
        id: usize,
        conn_id: &str,
        client: Client,
        db: &str,
        window: &mut Window,
        cx: &mut Context<Humon>,
    ) -> Self {
        let editor = cx.new(|cx| {
            let state = shell_editor(window, cx);
            state.focus(window, cx);
            state
        });
        let results = cx.new(|cx| shell_editor(window, cx));
        let sub = cx.subscribe_in(
            &editor,
            window,
            move |this: &mut Humon, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { secondary: true, .. } => this.run_console(id, window, cx),
                InputEvent::Change => {
                    let input = input.clone();
                    if let Some(console) = this.console_mut(id) {
                        // The editor pairs brackets itself; only completion and wrapping run here.
                        console.smart.on_change(&input, false, window, cx);
                    }
                }
                _ => {}
            },
        );
        let (handle, start_error) = match EngineHandle::spawn(Some(client), db.to_string()) {
            Ok(h) => (Some(h), None),
            Err(e) => (None, Some(e)),
        };
        if let Some(e) = start_error {
            results.update(cx, |s, cx| s.set_value(format!("// ✖ {e}"), window, cx));
        }
        Self {
            id,
            conn_id: conn_id.to_string(),
            db: db.to_string(),
            handle,
            editor,
            results,
            smart: Smart::default(),
            recall: Recall::default(),
            entries: Vec::new(),
            running: false,
            history_open: false,
            _subscriptions: vec![sub],
        }
    }
}

impl Humon {
    pub fn console_mut(&mut self, id: usize) -> Option<&mut ConsoleTab> {
        self.consoles.iter_mut().find(|c| c.id == id)
    }

    /// Replaces the editor's text, e.g. from history or a collection's Shell button.
    pub fn set_console_input(&mut self, id: usize, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(console) = self.console_mut(id) else { return };
        console.smart.reset(&text);
        console.input_focus(window, cx, text);
    }

    /// Redraws the results pane and scrolls to its end.
    fn show_results(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(console) = self.console_mut(id) else { return };
        let text = transcript(&console.entries);
        let last = text.lines().count().saturating_sub(1) as u32;
        console.results.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.set_cursor_position(
                Position {
                    line: last,
                    character: 0,
                },
                window,
                cx,
            );
        });
    }

    /// Runs the selection, or the whole editor when nothing is selected. The
    /// code stays in the editor, as in IntelliShell.
    pub fn run_console(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(console) = self.console_mut(id) else { return };
        let state = console.editor.read(cx);
        let selected = state.selected_value().trim().to_string();
        let code = if selected.is_empty() {
            state.value().trim().to_string()
        } else {
            selected
        };
        if code.is_empty() || console.running {
            return;
        }
        let Some(handle) = &console.handle else { return };
        let rx = handle.eval(code.clone());
        console.recall.push(&code);
        console.history_open = false;
        console.entries.push(Entry {
            db: console.db.clone(),
            code,
            lines: vec![Line {
                text: t!("console.running").into(),
                kind: LineKind::Info,
            }],
        });
        console.running = true;
        self.show_results(id, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let reply = rx.await;
            this.update_in(cx, |this, window, cx| {
                if let Some(console) = this.console_mut(id) {
                    console.running = false;
                    if let Ok(reply) = reply {
                        console.db = reply.db;
                        if let Some(entry) = console.entries.last_mut() {
                            entry.lines = lines(&reply.outputs);
                        }
                    }
                }
                this.show_results(id, window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// ⌥↑/⌥↓: earlier and later commands, keeping what was being typed.
    fn console_history(&mut self, id: usize, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(console) = self.console_mut(id) else { return };
        let current = console.editor.read(cx).value().to_string();
        let text = if back {
            console.recall.prev(&current)
        } else {
            console.recall.next()
        };
        if let Some(text) = text {
            self.set_console_input(id, text, window, cx);
        }
    }

    pub fn render_console(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let console = &self.consoles[ix];
        let id = console.id;
        let theme = cx.theme();
        let (border, muted, panel, hover, bg) = (
            theme.border,
            theme.muted_foreground,
            theme.sidebar,
            theme.list_hover,
            theme.background,
        );
        let mono = theme.mono_font_family.clone();

        let toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .bg(panel)
            .child(
                Button::new(("console-run", id))
                    .primary()
                    .label(format!("▶ {}", t!("console.run")))
                    .disabled(console.running || console.handle.is_none())
                    .on_click(cx.listener(move |this, _, window, cx| this.run_console(id, window, cx))),
            )
            .when(console.running, |d| {
                d.child(
                    Button::new(("console-stop", id))
                        .danger()
                        .label(format!("■ {}", t!("console.stop")))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(handle) = this.console_mut(id).and_then(|c| c.handle.as_ref()) {
                                handle.stop();
                            }
                            cx.notify();
                        })),
                )
            })
            .child(
                Button::new(("console-history", id))
                    .label(t!("console.history"))
                    .when(console.history_open, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(console) = this.console_mut(id) {
                            console.history_open = !console.history_open;
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new(("console-clear", id))
                    .label(t!("console.clear"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(console) = this.console_mut(id) {
                            console.entries.clear();
                        }
                        this.show_results(id, window, cx);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("console.hint").to_string()),
            )
            .child(
                div()
                    .font_family(mono.clone())
                    .text_xs()
                    .text_color(muted)
                    .child(format!("db: {}", console.db)),
            );

        let history = console.history_open.then(|| {
            let mut list = div()
                .id(("console-history-list", id))
                .flex()
                .flex_col()
                .max_h(px(220.))
                .overflow_y_scroll()
                .border_b_1()
                .border_color(border)
                .bg(panel)
                .font_family(mono.clone())
                .text_xs();
            let commands: Vec<String> = console
                .recall
                .newest_first()
                .take(HISTORY_ROWS)
                .map(str::to_string)
                .collect();
            if commands.is_empty() {
                list = list.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(muted)
                        .child(t!("console.no_history").to_string()),
                );
            }
            for (row, command) in commands.into_iter().enumerate() {
                let first_line = command.lines().next().unwrap_or_default().to_string();
                list = list.child(
                    div()
                        .id(("console-history-row", row))
                        .px_3()
                        .py_1()
                        .truncate()
                        .cursor_pointer()
                        .hover(|s| s.bg(hover))
                        .child(first_line)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(console) = this.console_mut(id) {
                                console.history_open = false;
                            }
                            this.set_console_input(id, command.clone(), window, cx);
                            cx.notify();
                        })),
                );
            }
            list
        });

        let editor_lines = console.editor.read(cx).value().lines().count().max(1);
        div()
            .id(("console", id))
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(move |this, _: &HistoryPrev, window, cx| this.console_history(id, true, window, cx)))
            .on_action(
                cx.listener(move |this, _: &HistoryNext, window, cx| this.console_history(id, false, window, cx)),
            )
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .children(history)
            .child(
                div()
                    .flex_none()
                    .border_b_1()
                    .border_color(border)
                    .child(Editor::new(&console.editor).h(px((editor_lines as f32 * 18. + 24.).clamp(120., 320.)))),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .bg(bg)
                    .child(Editor::new(&console.results).readonly(true).h(relative(1.))),
            )
    }
}

impl ConsoleTab {
    fn input_focus(&self, window: &mut Window, cx: &mut Context<Humon>, text: String) {
        self.editor.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.focus(window, cx);
        });
    }
}
