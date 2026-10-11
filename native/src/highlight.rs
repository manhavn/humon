//! A small lexer for mongo shell syntax. The same token kinds color the
//! document view and the editor, so both always agree. Kind names are the
//! theme's syntax names ("string", "number", ...), so colors come from the theme.

use std::ops::Range;

use gpui_kit::component::input::{FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Rope};
use gpui_kit::{Context, HighlightStyle, SharedString, Window};
use gpui_kit::component::input::EditorState;

pub type Token = (Range<usize>, &'static str);

const HELPERS: &[&str] = &[
    "ObjectId", "ObjectID", "ISODate", "Date", "NumberInt", "NumberLong", "NumberDecimal",
    "Int32", "Int64", "Long", "Decimal128", "UUID", "BinData", "Timestamp", "RegExp", "MinKey", "MaxKey", "new",
];

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Whether the next non-space char after `end` is `:`, which makes a token a key.
fn followed_by_colon(text: &str, end: usize) -> bool {
    text[end..].trim_start().starts_with(':')
}

/// Whether a `/` at `pos` starts a regex rather than being stray punctuation.
fn regex_allowed(text: &str, pos: usize) -> bool {
    matches!(text[..pos].trim_end().chars().last(), None | Some(':' | ',' | '[' | '(' | '{'))
}

pub fn tokenize(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        let start = i;
        match c {
            '"' | '\'' => {
                i += 1;
                while i < text.len() {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b if b == c as u8 => {
                            i += 1;
                            break;
                        }
                        b'\n' => break,
                        _ => i += 1,
                    }
                }
                i = i.min(text.len());
                let kind = if followed_by_colon(text, i) { "property" } else { "string" };
                out.push((start..i, kind));
            }
            '/' if text[i..].starts_with("//") => {
                i = text[i..].find('\n').map_or(text.len(), |n| i + n);
                out.push((start..i, "comment"));
            }
            '/' if regex_allowed(text, i) => {
                i += 1;
                let mut in_class = false;
                while i < text.len() {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'[' => { in_class = true; i += 1 }
                        b']' => { in_class = false; i += 1 }
                        b'/' if !in_class => { i += 1; break }
                        b'\n' => break,
                        _ => i += 1,
                    }
                }
                while i < text.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                i = i.min(text.len());
                out.push((start..i, "string.regex"));
            }
            '0'..='9' | '-' | '.' if c != '-' && c != '.' || text[i + 1..].starts_with(|n: char| n.is_ascii_digit()) => {
                i += 1;
                while i < text.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || ((bytes[i] == b'-' || bytes[i] == b'+') && matches!(bytes[i - 1], b'e' | b'E'))) {
                    i += 1;
                }
                out.push((start..i, "number"));
            }
            c if c.is_alphabetic() || c == '_' || c == '$' => {
                while i < text.len() {
                    let ch = text[i..].chars().next().unwrap();
                    if !is_ident_char(ch) && ch != '.' {
                        break;
                    }
                    i += ch.len_utf8();
                }
                let word = &text[start..i];
                let kind = if followed_by_colon(text, i) {
                    "property"
                } else if matches!(word, "true" | "false") {
                    "boolean"
                } else if matches!(word, "null" | "undefined" | "Infinity" | "NaN") {
                    "constant"
                } else if HELPERS.contains(&word) {
                    "function"
                } else {
                    "variable"
                };
                out.push((start..i, kind));
            }
            '{' | '}' | '[' | ']' | '(' | ')' => {
                i += 1;
                out.push((start..i, "punctuation.bracket"));
            }
            ',' | ':' => {
                i += 1;
                out.push((start..i, "punctuation.delimiter"));
            }
            _ => i += c.len_utf8(),
        }
    }
    out
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

    fn styles(&self, range: &Range<usize>, resolver: &dyn HighlightStyleResolver) -> Vec<(Range<usize>, HighlightStyle)> {
        let mut runs = Vec::new();
        let mut pos = range.start;
        for (r, kind) in &self.tokens {
            if r.end <= range.start || r.start >= range.end {
                continue;
            }
            let (s, e) = (r.start.max(range.start), r.end.min(range.end));
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
    use super::*;

    fn kinds(text: &str) -> Vec<(&str, &'static str)> {
        tokenize(text).into_iter().map(|(r, k)| (&text[r], k)).collect()
    }

    #[test]
    fn shell_document() {
        let k = kinds(r#"{ _id: ObjectId("65a1"), "n": -2.5, ok: true, re: /a\/b/i, x: null }"#);
        assert!(k.contains(&("_id", "property")));
        assert!(k.contains(&("ObjectId", "function")));
        assert!(k.contains(&("\"65a1\"", "string")));
        assert!(k.contains(&("\"n\"", "property")));
        assert!(k.contains(&("-2.5", "number")));
        assert!(k.contains(&("true", "boolean")));
        assert!(k.contains(&("/a\\/b/i", "string.regex")));
        assert!(k.contains(&("null", "constant")));
    }

    #[test]
    fn unterminated_string_stops_at_line_end() {
        let k = kinds("{ a: \"abc\n b: 1 }");
        assert!(k.contains(&("\"abc", "string")));
        assert!(k.contains(&("b", "property")));
    }
}
