//! The filter, sort and projection boxes: one-line code editors with shell
//! colors and schema-based completion.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::input::{CompletionProvider, Editor, EditorState, InputHighlighter, Rope, RopeExt};
use gpui_kit::*;
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit, TextEdit,
};
use rust_i18n::t;

use crate::complete::{Kind, Mode, complete};
use crate::highlight::ShellHighlighter;
use crate::schema::Schema;

/// The tab's schema, filled in once sampling finishes; the editors read it on
/// each keystroke, so they pick it up without being rebuilt.
pub type SchemaCell = Rc<RefCell<Option<Rc<Schema>>>>;

pub fn query_editor(mode: Mode, schema: SchemaCell, window: &mut Window, cx: &mut Context<EditorState>) -> EditorState {
    let mut state = EditorState::new(window, cx)
        .language("mongosh")
        .line_number(false)
        .folding(false)
        .soft_wrap(false)
        .indent_guides(false)
        .submit_on_enter(true);
    state.set_highlighter_factory(
        Rc::new(|_| Some(Box::new(ShellHighlighter::default()) as Box<dyn InputHighlighter>)),
        cx,
    );
    state.lsp_mut().completion_provider = Some(Rc::new(QueryCompleter { mode, schema }));
    state
}

/// A query editor sized like a one-line input. The editor pads its text 8px
/// top and bottom; a 14px row fills the rest of the 32px exactly, centered.
pub(crate) fn query_box(state: &Entity<EditorState>) -> impl IntoElement {
    Editor::new(state).h(px(32.)).line_height(px(14.))
}

struct QueryCompleter {
    mode: Mode,
    schema: SchemaCell,
}

impl CompletionProvider for QueryCompleter {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let Some(schema) = self.schema.borrow().clone() else {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        };
        let found = complete(&text.to_string(), offset, self.mode, &schema);
        let range = lsp_types::Range {
            start: text.offset_to_position(found.replace.start),
            end: text.offset_to_position(found.replace.end),
        };
        let items = found
            .items
            .into_iter()
            .map(|s| CompletionItem {
                kind: Some(match s.kind {
                    Kind::Field => CompletionItemKind::FIELD,
                    Kind::Operator => CompletionItemKind::OPERATOR,
                    Kind::Value => CompletionItemKind::VALUE,
                }),
                detail: match s.kind {
                    Kind::Operator => Some(t!(&s.detail).to_string()),
                    _ if s.detail.is_empty() => None,
                    _ => Some(s.detail),
                },
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: s.insert,
                })),
                label: s.label,
                ..Default::default()
            })
            .collect();
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        new_text
            .chars()
            .any(|c| c.is_alphanumeric() || matches!(c, '$' | '_' | '.' | '"' | '\'' | '{' | ',' | ' '))
    }
}
