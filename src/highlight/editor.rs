//! The lexer wired into gpui's code editor: an `InputHighlighter` and a
//! ready-made shell editor.

use std::ops::Range;

use gpui_kit::component::input::EditorState;
use gpui_kit::component::input::{FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Rope, RopeExt};
use gpui_kit::{Context, HighlightStyle, SharedString, Window};
use lsp_types::{CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit, TextEdit};
use rust_i18n::t;

use crate::complete::check::{Problem, Severity};
use crate::complete::{Completions, Kind, Suggestion};

use super::{Token, tokenize};

/// A code editor with shell syntax colors, for editing or (read-only) for
/// showing text that should be selectable and searchable.
pub fn shell_editor(window: &mut Window, cx: &mut Context<EditorState>) -> EditorState {
    let mut state = EditorState::new(window, cx)
        .language("mongosh")
        .line_number(false)
        .folding(false);
    state.set_highlighter_factory(
        std::rc::Rc::new(|_| Some(Box::new(ShellHighlighter::default()) as Box<dyn InputHighlighter>)),
        cx,
    );
    state
}

/// Typing these may open (or refine) the completion menu.
pub fn is_completion_trigger(new_text: &str) -> bool {
    new_text
        .chars()
        .any(|c| c.is_alphanumeric() || matches!(c, '$' | '_' | '.' | '"' | '\'' | '{' | ',' | ' '))
}

/// Suggestions as the editor's completion menu takes them.
pub fn completion_response(text: &Rope, found: Completions) -> CompletionResponse {
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
            detail: menu_detail(&s),
            // The full summary and the docs link, beside the menu for the selected item.
            documentation: s
                .link
                .map(|link| lsp_types::Documentation::String(format!("{}\n\n{link}", s.detail))),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range,
                new_text: s.insert,
            })),
            label: s.label,
            ..Default::default()
        })
        .collect();
    CompletionResponse::Array(items)
}

/// Underlines `problems` (byte ranges in the editor's text) with their messages.
pub fn show_problems(state: &mut EditorState, problems: Vec<Problem>, cx: &mut Context<EditorState>) {
    let text = state.text().clone();
    let Some(set) = state.diagnostics_mut() else { return };
    set.reset(&text);
    for p in problems {
        let end = p.range.end.max(p.range.start + 1).min(text.len());
        let start = p.range.start.min(end);
        set.push(lsp_types::Diagnostic {
            range: lsp_types::Range {
                start: text.offset_to_position(start),
                end: text.offset_to_position(end),
            },
            severity: Some(match p.severity {
                Severity::Error => lsp_types::DiagnosticSeverity::ERROR,
                Severity::Warning => lsp_types::DiagnosticSeverity::WARNING,
            }),
            message: p.message.text(),
            ..Default::default()
        });
    }
    cx.notify();
}

/// The grey text beside a suggestion: a translated short meaning when there
/// is one, else the detail cut to fit the menu.
fn menu_detail(s: &Suggestion) -> Option<String> {
    const MAX: usize = 60;
    let text = match s.detail_key {
        Some(key) => t!(key).to_string(),
        None => s.detail.clone(),
    };
    match text.chars().count() {
        0 => None,
        n if n > MAX => Some(text.chars().take(MAX - 1).collect::<String>() + "…"),
        _ => Some(text),
    }
}

/// Editor highlighter: re-lexes the whole buffer on each edit, which is cheap
/// at document sizes.
#[derive(Default)]
pub struct ShellHighlighter {
    tokens: Vec<Token>,
}

impl InputHighlighter for ShellHighlighter {
    fn language(&self) -> SharedString {
        "mongosh".into()
    }

    fn update(&mut self, _: Option<InputEdit>, text: &Rope, _: bool, _: &mut Window, _: &mut Context<EditorState>) {
        self.tokens = tokenize(&text.to_string());
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let mut runs = Vec::new();
        let mut pos = range.start;
        for (r, kind) in &self.tokens {
            if r.end <= range.start || r.start >= range.end {
                continue;
            }
            let (s, e) = (r.start.max(range.start), r.end.min(range.end));
            if s >= e {
                // An empty range that falls inside a token yields no run.
                continue;
            }
            if s > pos {
                runs.push((pos..s, HighlightStyle::default()));
            }
            runs.push((s..e, resolver.style(kind).unwrap_or_default()));
            pos = e;
        }
        if pos < range.end {
            runs.push((pos..range.end, HighlightStyle::default()));
        }
        runs
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{ShellHighlighter, tokenize};
    use gpui_kit::HighlightStyle;
    use gpui_kit::component::input::{HighlightStyleResolver, InputHighlighter};

    /// gpui's editor needs style runs that are ordered, don't overlap and
    /// cover the requested range exactly; anything else renders garbage.
    #[test]
    fn styles_cover_any_range_exactly() {
        struct NoStyles;
        impl HighlightStyleResolver for NoStyles {
            fn style(&self, _: &str) -> Option<HighlightStyle> {
                Some(HighlightStyle::default())
            }
        }
        let samples = [
            "",
            "{ a: 1 }",
            "{ _id: ObjectId(\"65a1\"), re: /x\\/y/i, s: 'Çağrı 🍃', n: -1.5e3 }",
            "{ unterminated: \"abc\n b: [1, 2, { c: null }] }",
            "// comment\n{ a: true }",
        ];
        for text in samples {
            let h = ShellHighlighter { tokens: tokenize(text) };
            let bounds: Vec<usize> = (0..=text.len()).filter(|&i| text.is_char_boundary(i)).collect();
            for &start in &bounds {
                for &end in bounds.iter().filter(|&&e| e >= start) {
                    let runs = h.styles(&(start..end), &NoStyles);
                    let mut pos = start;
                    for (r, _) in &runs {
                        assert_eq!(r.start, pos, "gap or overlap in {text:?} for {start}..{end}: {runs:?}");
                        assert!(r.end > r.start, "empty run in {text:?}");
                        pos = r.end;
                    }
                    assert_eq!(pos, end, "runs don't reach the end in {text:?} for {start}..{end}");
                }
            }
        }
    }
}
