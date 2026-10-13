//! The shell tab, laid out like Studio 3T's IntelliShell: a multi-line
//! editor on top, and below it one block per run: the query in a header with
//! copy and run-again buttons, and its result in a read-only editor that can
//! be selected, copied and searched.

use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::{ActiveTheme, Disableable, resizable_panel, v_resizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mongodb::Client;
use rust_i18n::t;

use crate::app::Humon;
use std::cell::RefCell;
use std::rc::Rc;

use crate::complete::check::check;
use crate::complete::shell::arguments;
use crate::console::complete::ShellCompleter;
use crate::console::engine::EngineHandle;
use crate::console::lines::{Entry, Line, LineKind, body, failed, lines, result_text, summary, transcript};
use crate::console::recall::Recall;
use crate::highlight::{shell_editor, show_problems};
use crate::smart::Smart;
use crate::ui;

gpui_kit::actions!(humon_shell, [HistoryPrev, HistoryNext]);

/// The key context for the shell's own shortcuts (⌥↑/⌥↓).
pub const KEY_CONTEXT: &str = "HumonShell";

/// Commands the history menu lists.
const HISTORY_ROWS: usize = 20;
/// Runs kept on screen; older ones drop off the top.
const MAX_BLOCKS: usize = 100;

/// One run: the command, its output, and the editor showing the output.
struct Block {
    id: usize,
    entry: Entry,
    editor: Entity<EditorState>,
    started: std::time::Instant,
    took: Option<std::time::Duration>,
    /// Folded blocks show only their header; a new run folds the others.
    open: bool,
}

pub struct ConsoleTab {
    pub id: usize,
    pub conn_id: String,
    /// The database `db` points at, shown in the toolbar.
    pub db: String,
    /// The same, for completion to read.
    completion_db: Rc<RefCell<String>>,
    completer: Rc<ShellCompleter>,
    handle: Option<EngineHandle>,
    editor: Entity<EditorState>,
    blocks: Vec<Block>,
    next_block: usize,
    /// Keeps the newest block in view.
    scroll: ScrollHandle,
    /// Why the shell couldn't start, shown in place of results.
    start_error: Option<String>,
    smart: Smart,
    recall: Recall,
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
        sampling: crate::schema::Sampling,
        window: &mut Window,
        cx: &mut Context<Humon>,
    ) -> Self {
        let completion_db = Rc::new(RefCell::new(db.to_string()));
        let completer = Rc::new(ShellCompleter::new(client.clone(), completion_db.clone(), sampling));
        let editor = cx.new(|cx| {
            let mut state = shell_editor(window, cx);
            state.lsp_mut().completion_provider = Some(completer.clone());
            state.focus(window, cx);
            state
        });
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
                        console.check(cx);
                    }
                    this.session_dirty = true;
                }
                _ => {}
            },
        );
        let (handle, start_error) = match EngineHandle::spawn(Some(client), db.to_string()) {
            Ok(h) => (Some(h), None),
            Err(e) => (None, Some(e)),
        };
        Self {
            id,
            conn_id: conn_id.to_string(),
            db: db.to_string(),
            completion_db,
            completer,
            handle,
            editor,
            blocks: Vec::new(),
            next_block: 0,
            scroll: ScrollHandle::new(),
            start_error,
            smart: Smart::default(),
            recall: Recall::default(),
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
        let entry = Entry {
            db: console.db.clone(),
            code,
            lines: vec![Line {
                text: t!("console.running").into(),
                kind: LineKind::Info,
            }],
        };
        let text = body(&entry.lines);
        let editor = cx.new(|cx| {
            let mut state = shell_editor(window, cx);
            state.set_value(text, window, cx);
            state
        });
        for block in &mut console.blocks {
            block.open = false;
        }
        let block_id = console.next_block;
        console.blocks.push(Block {
            id: block_id,
            entry,
            editor,
            started: std::time::Instant::now(),
            took: None,
            open: true,
        });
        console.next_block += 1;
        if console.blocks.len() > MAX_BLOCKS {
            console.blocks.remove(0);
        }
        console.running = true;
        console.scroll.scroll_to_bottom();
        cx.spawn_in(window, async move |this, cx| {
            let reply = rx.await;
            this.update_in(cx, |this, window, cx| {
                if let Some(console) = this.console_mut(id) {
                    console.running = false;
                    if let Ok(reply) = reply {
                        console.completion_db.replace(reply.db.clone());
                        console.db = reply.db;
                        // By id: the block may have been deleted while running.
                        if let Some(block) = console.blocks.iter_mut().find(|b| b.id == block_id) {
                            block.entry.lines = lines(&reply.outputs);
                            block.took = Some(block.started.elapsed());
                            let text = body(&block.entry.lines);
                            block.editor.update(cx, |s, cx| s.set_value(text, window, cx));
                        }
                    }
                    console.scroll.scroll_to_bottom();
                }
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

        let toolbar = ui::bar(border)
            .bg(panel)
            .child(
                Button::new(("console-run", id))
                    .small()
                    .primary()
                    .label(format!("▶ {}", t!("console.run")))
                    .disabled(console.running || console.handle.is_none())
                    .on_click(cx.listener(move |this, _, window, cx| this.run_console(id, window, cx))),
            )
            .when(console.running, |d| {
                d.child(
                    Button::new(("console-stop", id))
                        .small()
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
                ui::tool(("console-history", id), t!("console.history"), console.history_open).on_click(cx.listener(
                    move |this, _, _, cx| {
                        if let Some(console) = this.console_mut(id) {
                            console.history_open = !console.history_open;
                        }
                        cx.notify();
                    },
                )),
            )
            .child(
                ui::tool(("console-clear", id), t!("console.clear"), false).on_click(cx.listener(
                    move |this, _, _, cx| {
                        if let Some(console) = this.console_mut(id) {
                            console.blocks.clear();
                        }
                        cx.notify();
                    },
                )),
            )
            .child(
                ui::tool(("console-copy-all", id), t!("console.copy_all"), false)
                    .disabled(console.blocks.is_empty())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(console) = this.console_mut(id) {
                            let entries: Vec<Entry> = console.blocks.iter().map(|b| b.entry.clone()).collect();
                            cx.write_to_clipboard(ClipboardItem::new_string(transcript(&entries)));
                        }
                    })),
            )
            .child(ui::note(t!("console.hint"), muted).flex_1().truncate())
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

        let danger = cx.theme().danger;
        // The editor and the results share the height; the divider between
        // them is dragged to make the editor taller or shorter.
        let blocks = div()
            .id(("console-blocks", id))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&console.scroll)
            .bg(bg)
            .when_some(console.start_error.clone(), |d, e| {
                d.child(div().px_3().py_2().text_color(danger).child(format!("✖ {e}")))
            })
            .children(console.blocks.iter().map(|b| self.render_block(id, b, cx)));
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
                div().flex_1().min_h_0().child(
                    v_resizable(("console-split", id))
                        .child(
                            resizable_panel()
                                .size(px(200.))
                                .size_range(px(80.)..px(800.))
                                .child(Editor::new(&console.editor).size_full()),
                        )
                        .child(resizable_panel().child(blocks)),
                ),
            )
    }

    /// One run: a header with the query and, on hover, copy and run-again;
    /// below it the result. Failed runs get a red edge.
    fn render_block(&self, id: usize, block: &Block, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted, panel, hover, danger, bg) = (
            theme.border,
            theme.muted_foreground,
            theme.sidebar,
            theme.list_hover,
            theme.danger,
            theme.background,
        );
        let mono = theme.mono_font_family.clone();
        let block_id = block.id;
        let group: SharedString = format!("block-{id}-{block_id}").into();
        // Header buttons don't also fold the block they sit on.
        let action = |key: &str, label: String| {
            div()
                .id(SharedString::from(format!("{key}-{id}-{block_id}")))
                .px_2()
                .border_1()
                .border_color(border)
                .rounded_sm()
                .bg(bg)
                .cursor_pointer()
                .hover(|s| s.bg(hover))
                .child(label)
        };
        let result = result_text(&block.entry.lines);
        let code = block.entry.code.clone();
        let again = code.clone();
        let actions = div()
            .flex_none()
            .flex()
            .gap_1()
            .opacity(0.)
            .group_hover(group.clone(), |s| s.opacity(1.))
            .child(
                action("copy-result", t!("console.copy_result").into()).on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    cx.write_to_clipboard(ClipboardItem::new_string(result.clone()));
                }),
            )
            .child(
                action("copy-query", t!("console.copy_query").into()).on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                }),
            )
            .child(
                action("run-again", t!("console.run_again").into()).on_click(cx.listener(
                    move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.set_console_input(id, again.clone(), window, cx);
                        this.run_console(id, window, cx);
                    },
                )),
            )
            .child(
                action("delete-block", t!("console.delete").into()).on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(console) = this.console_mut(id) {
                        console.blocks.retain(|b| b.id != block_id);
                    }
                    cx.notify();
                })),
            );
        let took = match block.took {
            Some(d) => crate::tab::status::format_duration(d),
            None => t!("console.running").into(),
        };
        let lines = block.editor.read(cx).value().lines().count().max(1);
        let header = div()
            .id(SharedString::from(format!("block-header-{id}-{block_id}")))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .bg(panel)
            .text_xs()
            .cursor_pointer()
            .hover(|s| s.bg(hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(block) = this
                    .console_mut(id)
                    .and_then(|c| c.blocks.iter_mut().find(|b| b.id == block_id))
                {
                    block.open = !block.open;
                }
                cx.notify();
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(10.))
                    .text_color(muted)
                    .child(if block.open { "▾" } else { "▸" }),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(muted)
                    .child(format!("{}>", block.entry.db)),
            )
            .child(div().flex_1().min_w_0().truncate().child(summary(&block.entry.code)))
            .child(actions)
            .child(div().flex_none().text_color(muted).child(took));
        div()
            .id(SharedString::from(format!("block-{id}-{block_id}")))
            .group(group)
            .flex()
            .border_b_1()
            .border_color(border)
            .font_family(mono)
            .child(
                div()
                    .w(px(3.))
                    .flex_none()
                    .when(failed(&block.entry.lines), |d| d.bg(danger)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(header)
                    .when(block.open, |d| {
                        d.child(
                            Editor::new(&block.editor)
                                .readonly(true)
                                .h(px((lines as f32 * 18. + 16.).clamp(34., 480.))),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl ConsoleTab {
    /// What a later launch needs to open this shell again.
    pub fn saved(&self, cx: &App) -> crate::session::SavedTab {
        crate::session::SavedTab::Shell {
            conn_id: self.conn_id.clone(),
            db: self.db.clone(),
            code: self.editor.read(cx).value().to_string(),
        }
    }

    /// Underlines problems in every MQL argument of the script, against the
    /// schemas completion has fetched (operators are checked without one).
    fn check(&self, cx: &mut Context<Humon>) {
        let text = self.editor.read(cx).value().to_string();
        let mut problems = Vec::new();
        for arg in arguments(&text) {
            let schema = self
                .completer
                .cached_schema(arg.db.as_deref(), &arg.coll)
                .unwrap_or_default();
            for mut p in check(&text[arg.range.clone()], arg.mode, &schema) {
                p.range = p.range.start + arg.range.start..p.range.end + arg.range.start;
                problems.push(p);
            }
        }
        self.editor.update(cx, |s, cx| show_problems(s, problems, cx));
    }

    fn input_focus(&self, window: &mut Window, cx: &mut Context<Humon>, text: String) {
        self.editor.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.focus(window, cx);
        });
    }
}
