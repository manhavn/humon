//! The shell tab: an output log and an input editor wired to an engine.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Editor, EditorState, InputEvent, InputHighlighter};
use gpui_kit::component::{ActiveTheme, Disableable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use mongodb::Client;
use rust_i18n::t;

use crate::app::Humon;
use crate::console::engine::EngineHandle;
use crate::console::lines::{Line, LineKind, lines};
use crate::console::recall::Recall;
use crate::highlight::{ShellHighlighter, tokenize};
use crate::smart::Smart;

/// One command and what it printed.
struct Entry {
    db: String,
    code: String,
    lines: Vec<Line>,
}

pub struct ConsoleTab {
    pub id: usize,
    pub conn_id: String,
    /// The database `db` points at, shown in the prompt.
    pub db: String,
    handle: Option<EngineHandle>,
    start_error: Option<String>,
    input: Entity<EditorState>,
    smart: Smart,
    recall: Recall,
    entries: Vec<Entry>,
    running: bool,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl ConsoleTab {
    pub fn new(id: usize, conn_id: &str, client: Client, db: &str, window: &mut Window, cx: &mut Context<Humon>) -> Self {
        let input = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language("mongosh")
                .line_number(false)
                .folding(false)
                .submit_on_enter(true);
            state.set_highlighter_factory(Rc::new(|_| Some(Box::new(ShellHighlighter::default()) as Box<dyn InputHighlighter>)), cx);
            state.focus(window, cx);
            state
        });
        let sub = cx.subscribe_in(&input, window, move |this: &mut Humon, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { shift: false, .. } => this.run_console(id, window, cx),
            InputEvent::Change => {
                let input = input.clone();
                if let Some(console) = this.console_mut(id) {
                    // The editor pairs brackets itself; only completion and wrapping run here.
                    console.smart.on_change(&input, false, window, cx);
                }
            }
            _ => {}
        });
        let (handle, start_error) = match EngineHandle::spawn(Some(client), db.to_string()) {
            Ok(h) => (Some(h), None),
            Err(e) => (None, Some(e)),
        };
        Self {
            id,
            conn_id: conn_id.to_string(),
            db: db.to_string(),
            handle,
            start_error,
            input,
            smart: Smart::default(),
            recall: Recall::default(),
            entries: Vec::new(),
            running: false,
            scroll: ScrollHandle::new(),
            _subscriptions: vec![sub],
        }
    }
}

impl Humon {
    pub fn console_mut(&mut self, id: usize) -> Option<&mut ConsoleTab> {
        self.consoles.iter_mut().find(|c| c.id == id)
    }

    fn set_console_input(&mut self, id: usize, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(console) = self.console_mut(id) else { return };
        console.smart.reset(&text);
        console.input.update(cx, |s, cx| s.set_value(text, window, cx));
    }

    pub fn run_console(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(console) = self.console_mut(id) else { return };
        let code = console.input.read(cx).value().trim().to_string();
        if code.is_empty() || console.running {
            return;
        }
        let Some(handle) = &console.handle else { return };
        let rx = handle.eval(code.clone());
        console.recall.push(&code);
        console.entries.push(Entry { db: console.db.clone(), code, lines: Vec::new() });
        console.running = true;
        console.scroll.scroll_to_bottom();
        self.set_console_input(id, String::new(), window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let reply = rx.await;
            this.update(cx, |this, cx| {
                if let Some(console) = this.console_mut(id) {
                    console.running = false;
                    if let Ok(reply) = reply {
                        console.db = reply.db;
                        if let Some(entry) = console.entries.last_mut() {
                            entry.lines = lines(&reply.outputs);
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

    /// ↑/↓ walk the command history while the input is a single line.
    fn console_key(&mut self, id: usize, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        if key != "up" && key != "down" {
            return;
        }
        let Some(console) = self.console_mut(id) else { return };
        let current = console.input.read(cx).value().to_string();
        if current.contains('\n') {
            return;
        }
        let text = if key == "up" { console.recall.prev(&current) } else { console.recall.next() };
        if let Some(text) = text {
            cx.stop_propagation();
            self.set_console_input(id, text, window, cx);
        }
    }

    pub fn render_console(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let console = &self.consoles[ix];
        let id = console.id;
        let theme = cx.theme();
        let (border, muted, danger, panel, fg) = (theme.border, theme.muted_foreground, theme.danger, theme.sidebar, theme.foreground);
        let mono = theme.mono_font_family.clone();
        let syntax = theme.highlight_theme.clone();
        let colored = |text: &str| {
            let highlights: Vec<_> = tokenize(text).into_iter().filter_map(|(r, k)| syntax.style(k).map(|s| (r, s))).collect();
            StyledText::new(text.to_string()).with_highlights(highlights)
        };

        let mut log = div()
            .id(("console-log", id))
            .track_scroll(&console.scroll)
            .flex_1()
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_1()
            .font_family(mono.clone())
            .text_xs();
        if let Some(e) = &console.start_error {
            log = log.child(div().text_color(danger).child(e.clone()));
        }
        for entry in &console.entries {
            for (i, code_line) in entry.code.lines().enumerate() {
                let prompt = if i == 0 { format!("{}>", entry.db) } else { "…".into() };
                log = log.child(
                    div().flex().gap_2().whitespace_nowrap().child(div().text_color(muted).child(prompt)).child(colored(code_line)),
                );
            }
            for line in &entry.lines {
                let row = div().whitespace_nowrap().min_h(px(16.));
                log = log.child(match line.kind {
                    LineKind::Value => row.child(colored(&line.text)),
                    LineKind::Print => row.text_color(fg).child(line.text.clone()),
                    LineKind::Info => row.text_color(muted).child(line.text.clone()),
                    LineKind::Error => row.text_color(danger).child(line.text.clone()),
                });
            }
        }
        if console.running {
            log = log.child(div().text_color(muted).child(t!("console.running").to_string()));
        }

        let lines_in_input = console.input.read(cx).value().lines().count().clamp(1, 10);
        let input_row = div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .border_t_1()
            .border_color(border)
            .bg(panel)
            .child(
                div()
                    .id(("console-input", id))
                    .flex()
                    .gap_2()
                    .items_start()
                    .capture_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| this.console_key(id, ev, window, cx)))
                    .child(div().pt_1().font_family(mono).text_xs().text_color(muted).child(format!("{}>", console.db)))
                    .child(div().flex_1().child(Editor::new(&console.input).h(px(lines_in_input as f32 * 18. + 14.)))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().text_xs().text_color(muted).child(t!("console.hint").to_string()))
                    .child(Button::new(("console-clear", id)).label(t!("console.clear")).on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(console) = this.console_mut(id) {
                            console.entries.clear();
                        }
                        cx.notify();
                    })))
                    .when(console.running, |d| {
                        d.child(Button::new(("console-stop", id)).danger().label(t!("console.stop")).on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(handle) = this.console_mut(id).and_then(|c| c.handle.as_ref()) {
                                handle.stop();
                            }
                            cx.notify();
                        })))
                    })
                    .child(
                        Button::new(("console-run", id))
                            .primary()
                            .label(t!("console.run"))
                            .disabled(console.running || console.handle.is_none())
                            .on_click(cx.listener(move |this, _, window, cx| this.run_console(id, window, cx))),
                    ),
            );

        div().flex_1().min_h_0().flex().flex_col().child(log).child(input_row)
    }
}
