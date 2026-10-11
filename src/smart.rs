//! Typing helpers: completes helper names
//! (`Obj` -> `ObjectId("|")`), wraps pasted or typed ids/dates, and for the
//! single-line query fields closes brackets and quotes (the code editor does
//! that itself).
//!
//! gpui inputs only report "the text changed", so each change is diffed
//! against the previous text to recover what was typed or pasted. The rules
//! are pure (`plan`) and the edit is applied through the input's undoable
//! `replace`.

use std::ops::Range;

use gpui_kit::base::input::{InputBaseState, InputModeKind};
use gpui_kit::{App, Entity, Window};

const HELPERS: [(&str, &str, &str); 7] = [
    ("ObjectId", "(\"", "\")"),
    ("ISODate", "(\"", "\")"),
    ("UUID", "(\"", "\")"),
    ("NumberDecimal", "(\"", "\")"),
    ("NumberLong", "(", ")"),
    ("NumberInt", "(", ")"),
    ("Timestamp", "(", ")"),
];
const MIN_PREFIX: usize = 3;

/// Per-input state: the last seen text, and after a completion the rest of
/// the name the user may keep typing out of habit (those keys are swallowed).
#[derive(Default)]
pub struct Smart {
    prev: String,
    pending: Option<(usize, String)>,
}

/// A replacement to make: `range` becomes `text`, then the caret goes to `caret`.
#[derive(Debug, PartialEq)]
pub struct Plan {
    pub range: Range<usize>,
    pub text: String,
    pub caret: usize,
}

fn pair(c: char) -> Option<char> {
    match c {
        '{' => Some('}'),
        '[' => Some(']'),
        '(' => Some(')'),
        '"' => Some('"'),
        '\'' => Some('\''),
        _ => None,
    }
}

/// Whether byte offset `pos` falls inside a quoted string.
pub fn in_string(text: &str, pos: usize) -> bool {
    let mut quote = None;
    let mut chars = text[..pos].chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(_) if c == '\\' => {
                chars.next();
            }
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None => {}
        }
    }
    quote.is_some()
}

/// The innermost bracket still open at `pos`, ignoring strings.
fn enclosing_opener(text: &str, pos: usize) -> Option<char> {
    let mut stack = Vec::new();
    let mut quote = None;
    let mut chars = text[..pos].chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(_) if c == '\\' => {
                chars.next();
            }
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => quote = Some(c),
                '{' | '[' | '(' => stack.push(c),
                '}' | ']' | ')' => {
                    stack.pop();
                }
                _ => {}
            },
        }
    }
    stack.pop()
}

/// Whether `pos` is where a value goes (after `:`, or inside an array or call), not a key.
fn is_value_position(text: &str, pos: usize) -> bool {
    match text[..pos].trim_end().chars().last() {
        Some(':' | '[' | '(') => true,
        Some(',') => enclosing_opener(text, pos) != Some('{'),
        _ => false,
    }
}

/// Shell form of a pasted literal, or None if it should stay as typed.
pub fn wrap_literal(text: &str) -> Option<String> {
    let hex = |s: &str| s.chars().all(|c| c.is_ascii_hexdigit());
    if text.len() == 24 && hex(text) {
        return Some(format!("ObjectId(\"{text}\")"));
    }
    let parts: Vec<&str> = text.split('-').collect();
    if parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12]) && parts.iter().all(|p| hex(p)) {
        return Some(format!("UUID(\"{text}\")"));
    }
    let b = text.as_bytes();
    let is_date = b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && (b.len() == 10 || matches!(b[10], b'T' | b' '))
        && text[10..].chars().all(|c| c.is_ascii_digit() || "T :.Z+-".contains(c));
    is_date.then(|| format!("ISODate(\"{text}\")"))
}

/// The helper a typed prefix stands for, if exactly one matches.
pub fn complete_helper(prefix: &str) -> Option<(&'static str, &'static str, &'static str)> {
    if prefix.len() < MIN_PREFIX {
        return None;
    }
    let p = prefix.to_lowercase();
    let mut matches = HELPERS.iter().filter(|(name, ..)| name.to_lowercase().starts_with(&p));
    match (matches.next(), matches.next()) {
        (Some(h), None) => Some(*h),
        _ => None,
    }
}

/// Byte range of the common prefix and suffix that `old` and `new` share.
fn diff(old: &str, new: &str) -> (usize, usize) {
    let prefix = old
        .char_indices()
        .zip(new.chars())
        .find(|((_, a), b)| a != b)
        .map_or(old.len().min(new.len()), |((i, _), _)| i);
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .scan(0, |acc, l| {
            *acc += l;
            Some(*acc)
        })
        .take_while(|&n| n <= max_suffix)
        .last()
        .unwrap_or(0);
    (prefix, suffix)
}

/// Where the edit from `old` to `new` happened, as (offset, removed, inserted).
/// Text alone is ambiguous (typing `'` before `'`), so a plain insertion or
/// deletion is located from the caret, which sits right after it.
fn locate<'a>(old: &'a str, new: &'a str, caret: usize) -> (usize, &'a str, &'a str) {
    let caret = caret.min(new.len());
    if new.len() > old.len() {
        let at = caret.saturating_sub(new.len() - old.len());
        if new.is_char_boundary(at) && old.is_char_boundary(at) && new[..at] == old[..at] && new[caret..] == old[at..] {
            return (at, "", &new[at..caret]);
        }
    } else if old.len() > new.len() {
        let end = caret + (old.len() - new.len());
        if old.is_char_boundary(caret) && old.is_char_boundary(end) && old[..caret] == new[..caret] && old[end..] == new[caret..] {
            return (caret, &old[caret..end], "");
        }
    }
    let (pre, suf) = diff(old, new);
    (pre, &old[pre..old.len() - suf], &new[pre..new.len() - suf])
}

impl Smart {
    /// Works out what the change from the last seen text calls for. `caret`
    /// is the cursor's byte offset after the change; `auto_close` adds
    /// bracket/quote pairing for inputs that lack it.
    pub fn plan(&mut self, text: &str, caret: usize, auto_close: bool) -> Option<Plan> {
        let prev = std::mem::replace(&mut self.prev, text.to_string());
        let (at, removed, inserted) = locate(&prev, text, caret);
        let typed = if removed.is_empty() && inserted.chars().count() == 1 { inserted.chars().next() } else { None };

        // Swallow the rest of a completed name typed out of habit.
        if let Some((pos, rest)) = self.pending.take()
            && let Some(c) = typed
            && at == pos
            && rest.chars().next().is_some_and(|r| r.eq_ignore_ascii_case(&c))
        {
            let rest: String = rest.chars().skip(1).collect();
            if !rest.is_empty() {
                self.pending = Some((pos, rest));
            }
            return Some(Plan { range: at..at + c.len_utf8(), text: String::new(), caret: at });
        }

        // Pasted literal outside a string.
        if removed.is_empty() && inserted.len() > 1 && !in_string(text, at) {
            let trimmed = inserted.trim();
            if let Some(wrapped) = wrap_literal(trimmed) {
                let caret = at + wrapped.len();
                return Some(Plan { range: at..at + inserted.len(), text: wrapped, caret });
            }
        }

        // Backspace between an empty pair removes both halves.
        if auto_close && inserted.is_empty() && removed.chars().count() == 1 {
            let opener = removed.chars().next().unwrap();
            if let Some(close) = pair(opener)
                && text[at..].starts_with(close)
            {
                return Some(Plan { range: at..at + close.len_utf8(), text: String::new(), caret: at });
            }
        }

        let c = typed?;
        let after = at + c.len_utf8();

        if auto_close {
            let next = text[after..].chars().next();
            // A closer typed in front of the same closer steps over it.
            let is_closer = matches!(c, '}' | ']' | ')') || (matches!(c, '"' | '\'') && in_string(&prev, at));
            if is_closer && next == Some(c) {
                return Some(Plan { range: after..after + c.len_utf8(), text: String::new(), caret: after });
            }
            if let Some(close) = pair(c)
                && !in_string(&prev, at)
            {
                let prev_char = text[..at].chars().last();
                let word_before = prev_char.is_some_and(|p| p.is_alphanumeric() || p == '\\');
                let next_ok = next.is_none_or(|n| n.is_whitespace() || "}]),:".contains(n));
                if next_ok && !(close == c && word_before) {
                    return Some(Plan { range: after..after, text: close.to_string(), caret: after });
                }
            }
        }

        // A bare ObjectId hex is finished by `,` `}` `]` or a space.
        if matches!(c, ',' | '}' | ']' | ' ') && !in_string(text, at) {
            let before = &text[..at];
            let token_start = at.saturating_sub(24);
            let token = &before[token_start..];
            let boundary = before[..token_start].chars().last().is_none_or(|p| !(p.is_alphanumeric() || p == '"' || p == '\'' || p == '_'));
            if token.len() == 24 && token.chars().all(|ch| ch.is_ascii_hexdigit()) && boundary {
                let wrapped = format!("ObjectId(\"{token}\")");
                let caret = token_start + wrapped.len() + c.len_utf8();
                return Some(Plan { range: token_start..at, text: wrapped, caret });
            }
        }

        // Helper name completion in value position.
        if c.is_ascii_alphabetic() {
            let before = &text[..after];
            let word_len = before.chars().rev().take_while(|ch| ch.is_ascii_alphabetic()).count();
            let start = after - word_len;
            let boundary = text[..start].chars().last().is_none_or(|p| !(p.is_alphanumeric() || "_$\"'.".contains(p)));
            let next_is_word = text[after..].chars().next().is_some_and(|n| n.is_alphanumeric());
            if boundary && !next_is_word && !in_string(text, start) && is_value_position(text, start)
                && let Some((name, open, close)) = complete_helper(&text[start..after])
            {
                let caret = start + name.len() + open.len();
                self.pending = Some((caret, format!("{}{open}", &name[word_len..])));
                return Some(Plan { range: start..after, text: format!("{name}{open}{close}"), caret });
            }
        }
        None
    }

    /// Records text set programmatically, so it isn't mistaken for typing.
    pub fn reset(&mut self, text: &str) {
        self.prev = text.to_string();
        self.pending = None;
    }

    /// Diffs the input's new text and applies any resulting edit to it.
    pub fn on_change<M: InputModeKind>(
        &mut self,
        input: &Entity<InputBaseState<M>>,
        auto_close: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (text, caret) = {
            let state = input.read(cx);
            (state.value().to_string(), state.cursor())
        };
        let Some(plan) = self.plan(&text, caret, auto_close) else { return };
        let mut result = text.clone();
        result.replace_range(plan.range.clone(), &plan.text);
        // Set before editing: the resulting change event must diff to nothing.
        self.prev = result;
        input.update(cx, |state, cx| {
            state.set_selected_range(plan.range.clone(), cx);
            state.replace(plan.text.clone(), window, cx);
            state.set_selected_range(plan.caret..plan.caret, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds keystrokes through `plan` like an input would, returning text with `|` at the caret.
    fn type_keys(keys: &str, auto_close: bool) -> String {
        let mut smart = Smart::default();
        let mut text = String::new();
        let mut caret = 0;
        for c in keys.chars() {
            text.insert(caret, c);
            caret += c.len_utf8();
            if let Some(p) = smart.plan(&text, caret, auto_close) {
                text.replace_range(p.range, &p.text);
                caret = p.caret;
                smart.prev = text.clone();
            }
        }
        format!("{}|{}", &text[..caret], &text[caret..])
    }

    fn paste(before: &str, clip: &str) -> String {
        let mut smart = Smart::default();
        smart.reset(before);
        let mut text = before.to_string();
        text.push_str(clip);
        let mut caret = text.len();
        if let Some(p) = smart.plan(&text, caret, true) {
            text.replace_range(p.range, &p.text);
            caret = p.caret;
        }
        format!("{}|{}", &text[..caret], &text[caret..])
    }

    #[test]
    fn auto_close_and_step_over() {
        assert_eq!(type_keys("{", true), "{|}");
        assert_eq!(type_keys("{ name: 'ali' }", true), "{ name: 'ali' }|");
        assert_eq!(type_keys("{ a: [1, 2] }", true), "{ a: [1, 2] }|");
        assert_eq!(type_keys("{ s: \"it's\" }", true), "{ s: \"it's\" }|");
        // The editor pairs brackets itself; without auto_close only completion runs.
        assert_eq!(type_keys("{", false), "{|");
    }

    #[test]
    fn backspace_removes_pair() {
        let mut smart = Smart::default();
        smart.reset("{}");
        assert_eq!(smart.plan("}", 0, true), Some(Plan { range: 0..1, text: String::new(), caret: 0 }));
    }

    #[test]
    fn completes_helpers_in_value_position() {
        assert_eq!(type_keys("{ _id: Obj", true), "{ _id: ObjectId(\"|\")}");
        assert_eq!(type_keys("{ _id: obj", true), "{ _id: ObjectId(\"|\")}");
        assert_eq!(type_keys("{ at: iso", true), "{ at: ISODate(\"|\")}");
        assert_eq!(type_keys("{ n: Num", true), "{ n: Num|}");
        assert_eq!(type_keys("{ n: NumberL", true), "{ n: NumberLong(|)}");
        assert_eq!(type_keys("{ a: [1, Obj", true), "{ a: [1, ObjectId(\"|\")]}");
        assert_eq!(type_keys("{ obj: 1 }", true), "{ obj: 1 }|");
        assert_eq!(type_keys("{ a: 1, obj: 2 }", true), "{ a: 1, obj: 2 }|");
        assert_eq!(type_keys("{ ok: true, x: null }", true), "{ ok: true, x: null }|");
    }

    #[test]
    fn habitual_full_typing_is_absorbed() {
        assert_eq!(type_keys("{ _id: ObjectId(\"x\") }", true), "{ _id: ObjectId(\"x\") }|");
        assert_eq!(type_keys("{ n: NumberLong(5) }", true), "{ n: NumberLong(5) }|");
        // The editor variant: closers are typed over by the editor, not by us.
        assert_eq!(type_keys("{ _id: Objx", false), "{ _id: ObjectId(\"x|\")");
    }

    #[test]
    fn wraps_ids_and_dates() {
        assert_eq!(paste("{ _id: ", "65a1b2c3d4e5f60718293a4b"), "{ _id: ObjectId(\"65a1b2c3d4e5f60718293a4b\")|");
        assert_eq!(paste("{ _id: \"", "65a1b2c3d4e5f60718293a4b"), "{ _id: \"65a1b2c3d4e5f60718293a4b|");
        assert_eq!(paste("{ u: ", "123e4567-e89b-12d3-a456-426614174000"), "{ u: UUID(\"123e4567-e89b-12d3-a456-426614174000\")|");
        assert_eq!(paste("{ at: ", "2024-01-02"), "{ at: ISODate(\"2024-01-02\")|");
        assert_eq!(type_keys("{ _id: 65a1b2c3d4e5f60718293a4b }", true), "{ _id: ObjectId(\"65a1b2c3d4e5f60718293a4b\") }|");
    }

    #[test]
    fn non_ascii_text_is_safe() {
        assert_eq!(type_keys("{ ad: 'Çağrı' }", true), "{ ad: 'Çağrı' }|");
    }
}
