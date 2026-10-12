//! A small lexer for mongo shell syntax. The same token kinds color the
//! document view and the editor, so both always agree. Kind names are the
//! theme's syntax names ("string", "number", ...), so colors come from the theme.

use std::ops::Range;

mod editor;

pub use editor::{ShellHighlighter, shell_editor};

pub type Token = (Range<usize>, &'static str);

const HELPERS: &[&str] = &[
    "ObjectId",
    "ObjectID",
    "ISODate",
    "Date",
    "NumberInt",
    "NumberLong",
    "NumberDecimal",
    "Int32",
    "Int64",
    "Long",
    "Decimal128",
    "UUID",
    "BinData",
    "Timestamp",
    "RegExp",
    "MinKey",
    "MaxKey",
    "new",
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
    matches!(
        text[..pos].trim_end().chars().last(),
        None | Some(':' | ',' | '[' | '(' | '{')
    )
}

const KEYWORDS: &[&str] = &[
    "const",
    "let",
    "var",
    "function",
    "return",
    "if",
    "else",
    "for",
    "while",
    "do",
    "break",
    "continue",
    "new",
    "typeof",
    "instanceof",
    "in",
    "of",
    "this",
    "throw",
    "try",
    "catch",
    "finally",
    "switch",
    "case",
    "default",
    "async",
    "await",
    "class",
    "delete",
    "void",
    "yield",
];

/// Colors `a.b.c` in code: `db` is special, a segment before `(` is a call,
/// other segments after a dot are properties.
fn identifiers(text: &str, start: usize, end: usize, out: &mut Vec<Token>) {
    let calls = text[end..].trim_start().starts_with('(');
    let mut pos = start;
    let segments: Vec<&str> = text[start..end].split('.').collect();
    for (n, word) in segments.iter().enumerate() {
        let range = pos..pos + word.len();
        pos += word.len() + 1;
        if word.is_empty() {
            continue;
        }
        let last = n == segments.len() - 1;
        let kind = if n == 0 && segments.len() == 1 && KEYWORDS.contains(word) {
            // `if (`, `for (`: keywords, though a paren follows.
            "keyword"
        } else if last && calls {
            "function"
        } else if n > 0 {
            "property"
        } else if *word == "db" {
            "variable.special"
        } else if matches!(*word, "true" | "false") {
            "boolean"
        } else if matches!(*word, "null" | "undefined" | "Infinity" | "NaN") {
            "constant"
        } else if KEYWORDS.contains(word) {
            "keyword"
        } else if HELPERS.contains(word) {
            "function"
        } else {
            "variable"
        };
        out.push((range, kind));
    }
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
                let kind = if followed_by_colon(text, i) {
                    "property"
                } else {
                    "string"
                };
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
                        b'[' => {
                            in_class = true;
                            i += 1
                        }
                        b']' => {
                            in_class = false;
                            i += 1
                        }
                        b'/' if !in_class => {
                            i += 1;
                            break;
                        }
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
            '0'..='9' | '-' | '.'
                if c != '-' && c != '.' || text[i + 1..].starts_with(|n: char| n.is_ascii_digit()) =>
            {
                i += 1;
                while i < text.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'.'
                        || ((bytes[i] == b'-' || bytes[i] == b'+') && matches!(bytes[i - 1], b'e' | b'E')))
                {
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
                // A trailing dot belongs to what follows (`db.` while typing).
                while text[start..i].ends_with('.') {
                    i -= 1;
                }
                if followed_by_colon(text, i) {
                    // A key, dotted paths included: `address.city: 1`.
                    out.push((start..i, "property"));
                } else {
                    identifiers(text, start, i, &mut out);
                }
            }
            '=' if text[i..].starts_with("=>") => {
                i += 2;
                out.push((start..i, "keyword"));
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
    fn numbers_comments_and_keys() {
        let k = kinds("// note\n{ n: 1e-3, m: .5, list: [1, -2], 'quoted': 1 }");
        assert!(k.contains(&("// note", "comment")));
        assert!(k.contains(&("1e-3", "number")));
        assert!(k.contains(&(".5", "number")));
        assert!(k.contains(&("-2", "number")));
        assert!(k.contains(&("'quoted'", "property")));
    }

    #[test]
    fn escapes_before_multibyte_chars_do_not_split_them() {
        for text in ["{ a: \"\\ç\" }", "{ r: /\\ğ/ }", "\"\\🍃", "/\\é"] {
            for (r, _) in tokenize(text) {
                assert!(
                    text.is_char_boundary(r.start) && text.is_char_boundary(r.end),
                    "{text:?}: {r:?}"
                );
            }
        }
    }

    #[test]
    fn shell_code_gets_colors_too() {
        let k = kinds("db.users.find({ age: 1 }).sort({ age: -1 })");
        assert!(k.contains(&("db", "variable.special")), "{k:?}");
        assert!(k.contains(&("users", "property")), "{k:?}");
        assert!(k.contains(&("find", "function")), "{k:?}");
        assert!(k.contains(&("sort", "function")), "{k:?}");
        assert!(k.contains(&("age", "property")), "{k:?}");

        let k = kinds("const names = db.users.find().map((u) => print(u.name))");
        assert!(k.contains(&("const", "keyword")), "{k:?}");
        assert!(k.contains(&("map", "function")), "{k:?}");
        assert!(k.contains(&("=>", "keyword")), "{k:?}");
        assert!(k.contains(&("print", "function")), "{k:?}");
        assert!(k.contains(&("name", "property")), "{k:?}");
        assert!(k.contains(&("names", "variable")), "{k:?}");

        let k = kinds("for (let i = 0; i < 3; i++) { if (i) { return new Date() } }");
        for word in ["for", "let", "if", "return", "new"] {
            assert!(k.contains(&(word, "keyword")), "{word}: {k:?}");
        }
        assert!(k.contains(&("Date", "function")), "{k:?}");
    }

    #[test]
    fn dotted_keys_stay_one_property() {
        let k = kinds("{ address.city: 1 }");
        assert!(k.contains(&("address.city", "property")), "{k:?}");
    }

    #[test]
    fn unterminated_string_stops_at_line_end() {
        let k = kinds("{ a: \"abc\n b: 1 }");
        assert!(k.contains(&("\"abc", "string")));
        assert!(k.contains(&("b", "property")));
    }
}
