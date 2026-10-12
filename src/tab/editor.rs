//! The editor for editing and inserting documents, and its save actions.

use super::*;

pub struct DocEditor {
    pub state: Entity<EditorState>,
    pub error: Option<String>,
    pub saving: bool,
    smart: Smart,
    _subscription: Subscription,
}

/// Which editor of a tab: the one editing a document, or the insert draft.
#[derive(Clone, Copy)]
pub(crate) enum EditorSlot {
    Edit,
    Draft,
}

/// An editor for shell-syntax documents in `slot` of tab `id`; ⌘↵ saves.
pub(crate) fn new_editor(
    text: String,
    id: usize,
    slot: EditorSlot,
    window: &mut Window,
    cx: &mut Context<Humon>,
) -> DocEditor {
    let mut smart = Smart::default();
    smart.reset(&text);
    let state = cx.new(|cx| {
        let mut state = EditorState::new(window, cx)
            .language("mongosh")
            .line_number(false)
            .folding(false)
            .default_value(text);
        state.set_highlighter_factory(
            Rc::new(|_| Some(Box::new(ShellHighlighter::default()) as Box<dyn InputHighlighter>)),
            cx,
        );
        state.focus(window, cx);
        state
    });
    let subscription = cx.subscribe_in(
        &state,
        window,
        move |this, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { secondary: true, .. } => match slot {
                EditorSlot::Edit => this.save_edit(id, window, cx),
                EditorSlot::Draft => this.save_draft(id, window, cx),
            },
            InputEvent::Change => {
                let input = input.clone();
                if let Some(editor) = this.editor_mut(id, slot) {
                    // The editor pairs brackets itself; only completion and wrapping run here.
                    editor.smart.on_change(&input, false, window, cx);
                }
            }
            _ => {}
        },
    );
    DocEditor {
        state,
        error: None,
        saving: false,
        smart,
        _subscription: subscription,
    }
}

impl Humon {
    pub(crate) fn editor_mut(&mut self, id: usize, slot: EditorSlot) -> Option<&mut DocEditor> {
        let tab = self.tab_mut(id)?;
        match slot {
            EditorSlot::Edit => tab.editing.as_mut().map(|(_, _, e)| e),
            EditorSlot::Draft => tab.draft.as_mut(),
        }
    }

    pub(crate) fn start_edit(&mut self, id: usize, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        let Some(doc_id) = tab.docs.get(ix).and_then(doc_id) else {
            return;
        };
        tab.card_error = None;
        // Edit the full stored document: the tab may show a projection, and
        // saving a projected copy would drop the hidden fields.
        let fut = tab.target.clone().get(doc_id.clone());
        self.run(fut, window, cx, move |this, res, window, cx| {
            let text = match res {
                Ok(Some(full)) => to_shell(&full, ""),
                Ok(None) => return this.set_card_error(id, ix, crate::mongo::gone()),
                Err(e) => return this.set_card_error(id, ix, e),
            };
            let editor = new_editor(text, id, EditorSlot::Edit, window, cx);
            if let Some(tab) = this.tab_mut(id) {
                tab.editing = Some((ix, doc_id, editor));
            }
        });
    }

    pub(crate) fn save_edit(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let Some((_, doc_id, editor)) = &mut tab.editing else {
            return;
        };
        if editor.saving {
            return;
        }
        editor.saving = true;
        editor.error = None;
        let text = editor.state.read(cx).value().to_string();
        let fut = tab.target.clone().replace(doc_id.clone(), text);
        self.run(fut, window, cx, move |this, res, window, cx| {
            let Some(tab) = this.tab_mut(id) else { return };
            match res {
                Ok(()) => {
                    tab.editing = None;
                    this.refresh(id, window, cx);
                }
                Err(e) => {
                    if let Some((_, _, editor)) = &mut tab.editing {
                        editor.saving = false;
                        editor.error = Some(e);
                    }
                }
            }
        });
    }

    pub(crate) fn open_draft(&mut self, id: usize, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let editor = new_editor(text, id, EditorSlot::Draft, window, cx);
        if let Some(tab) = self.tab_mut(id) {
            tab.draft = Some(editor);
        }
    }

    pub(crate) fn save_draft(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let Some(editor) = &mut tab.draft else { return };
        if editor.saving {
            return;
        }
        editor.saving = true;
        editor.error = None;
        let text = editor.state.read(cx).value().to_string();
        let fut = tab.target.clone().insert(text);
        self.run(fut, window, cx, move |this, res, window, cx| {
            let Some(tab) = this.tab_mut(id) else { return };
            match res {
                Ok(n) => {
                    tab.draft = None;
                    tab.notice = Some(t!("tab.inserted", n = n).into());
                    this.refresh(id, window, cx);
                }
                Err(e) => {
                    if let Some(editor) = &mut tab.draft {
                        editor.saving = false;
                        editor.error = Some(e);
                    }
                }
            }
        });
    }

    pub(crate) fn render_editor(
        &self,
        id: usize,
        editor: &DocEditor,
        save_label: SharedString,
        is_draft: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let lines = editor.state.read(cx).value().lines().count().clamp(6, 30);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(Editor::new(&editor.state).h(px(lines as f32 * 18. + 16.)))
            .when_some(editor.error.clone(), |d, e| {
                d.child(div().text_xs().text_color(theme.danger).child(e))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t!("tab.save_hint").to_string()),
                    )
                    .child(
                        Button::new(("cancel-edit", id))
                            .label(t!("tab.cancel"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(tab) = this.tab_mut(id) {
                                    if is_draft { tab.draft = None } else { tab.editing = None }
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("save-edit", id))
                            .primary()
                            .label(if editor.saving {
                                t!("tab.saving").into()
                            } else {
                                save_label
                            })
                            .disabled(editor.saving)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if is_draft {
                                    this.save_draft(id, window, cx)
                                } else {
                                    this.save_edit(id, window, cx)
                                }
                            })),
                    ),
            )
    }
}
