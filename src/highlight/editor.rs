//! The lexer wired into gpui's code editor: an `InputHighlighter` and a
//! ready-made shell editor.

use std::ops::Range;

use gpui_kit::component::input::EditorState;
use gpui_kit::component::input::{FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Rope};
use gpui_kit::{Context, HighlightStyle, SharedString, Window};

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
