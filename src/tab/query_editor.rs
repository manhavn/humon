//! The filter, sort and projection boxes: one-line code editors with shell
//! colors and schema-based completion.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::input::{CompletionProvider, Editor, EditorState, InputHighlighter, Rope};
use gpui_kit::*;
use lsp_types::{CompletionContext, CompletionResponse};

use crate::complete::{Mode, complete};
use crate::highlight::{ShellHighlighter, completion_response, is_completion_trigger};
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
        Task::ready(Ok(completion_response(text, found)))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        is_completion_trigger(new_text)
    }
}
