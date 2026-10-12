//! Suggestions for the filter, sort and projection boxes: what fits where
//! the cursor is, given the collection's sampled schema.

use std::ops::Range;

use crate::schema::{Field, Schema};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Filter,
    Sort,
    Project,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Field,
    Operator,
    Value,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Suggestion {
    pub label: String,
    pub insert: String,
    /// Types and presence for a field; for an operator, the i18n key of its meaning.
    pub detail: String,
    pub kind: Kind,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Completions {
    /// The text the chosen suggestion replaces.
    pub replace: Range<usize>,
    pub items: Vec<Suggestion>,
}

#[derive(Clone, Copy, PartialEq)]
enum Bracket {
    Object,
    Array,
}

struct Frame {
    bracket: Bracket,
    /// The last key read in this object.
    key: Option<String>,
    expect_key: bool,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$' || c == '.'
}

/// Reads `text` up to `end` and returns the open brackets, innermost last.
fn frames(text: &str, end: usize) -> Vec<Frame> {
    let src = &text[..end];
    let mut stack: Vec<Frame> = Vec::new();
    let mut chars = src.char_indices().peekable();
    let mut last_sig: Option<char> = None;
    while let Some((i, c)) = chars.next() {
        match c {
            '{' | '[' => stack.push(Frame {
                bracket: if c == '{' { Bracket::Object } else { Bracket::Array },
                key: None,
                expect_key: c == '{',
            }),
            '}' | ']' => {
                stack.pop();
            }
            ':' => {
                if let Some(top) = stack.last_mut().filter(|f| f.bracket == Bracket::Object) {
                    top.expect_key = false;
                }
            }
            ',' => {
                if let Some(top) = stack.last_mut().filter(|f| f.bracket == Bracket::Object) {
                    top.expect_key = true;
                    top.key = None;
                }
            }
            '"' | '\'' => {
                let mut value = String::new();
                let mut escaped = false;
                for (_, d) in chars.by_ref() {
                    if escaped {
                        value.push(d);
                        escaped = false;
                    } else if d == '\\' {
                        escaped = true;
                    } else if d == c {
                        break;
                    } else {
                        value.push(d);
                    }
                }
                set_key(&mut stack, value);
            }
            '/' if matches!(last_sig, None | Some(':' | ',' | '[' | '(')) => {
                // A regex literal: skip to its closing slash.
                let mut escaped = false;
                for (_, d) in chars.by_ref() {
                    if escaped {
                        escaped = false;
                    } else if d == '\\' {
                        escaped = true;
                    } else if d == '/' {
                        break;
                    }
                }
            }
            c if is_word_char(c) => {
                let mut stop = src.len();
                while let Some(&(j, d)) = chars.peek() {
                    if !is_word_char(d) {
                        stop = j;
                        break;
                    }
                    chars.next();
                }
                set_key(&mut stack, src[i..stop].to_string());
            }
            _ => {}
        }
        if !c.is_whitespace() {
            last_sig = Some(c);
        }
    }
    stack
}

fn set_key(stack: &mut [Frame], word: String) {
    if let Some(top) = stack
        .last_mut()
        .filter(|f| f.bracket == Bracket::Object && f.expect_key)
    {
        top.key = Some(word);
    }
}

/// Where the cursor is, in terms of what may be typed there.
#[derive(Debug, PartialEq)]
enum Slot {
    /// A field name; `prefix` is the path the names are under (inside `$elemMatch`).
    Field {
        prefix: String,
        root: bool,
    },
    /// An operator for this field.
    Operator(String),
    /// The value of this key.
    Value(String),
    Nothing,
}

fn slot(stack: &[Frame], mode: Mode) -> Slot {
    let Some(top) = stack.last() else {
        return Slot::Nothing;
    };
    if top.bracket == Bracket::Array {
        return Slot::Nothing;
    }
    if !top.expect_key {
        return top.key.clone().map_or(Slot::Nothing, Slot::Value);
    }
    // Keys of the enclosing objects, outermost first.
    let keys = stack[..stack.len() - 1]
        .iter()
        .filter(|f| f.bracket == Bracket::Object)
        .map(|f| f.key.as_deref().unwrap_or(""));
    if mode != Mode::Filter {
        return if stack.len() == 1 {
            Slot::Field {
                prefix: String::new(),
                root: true,
            }
        } else {
            Slot::Nothing
        };
    }
    let mut prefix = String::new();
    let mut field: Option<String> = None;
    let mut root = true;
    for key in keys {
        match key {
            "$and" | "$or" | "$nor" => field = None,
            "$elemMatch" => match field.take() {
                Some(f) => {
                    prefix = format!("{f}.");
                    root = false;
                }
                None => return Slot::Nothing,
            },
            "$not" => {}
            k if k.starts_with('$') || k.is_empty() => return Slot::Nothing,
            k => field = Some(format!("{prefix}{k}")),
        }
    }
    match field {
        Some(f) => Slot::Operator(f),
        None => Slot::Field { prefix, root },
    }
}

const LOGICAL: &[(&str, &str)] = &[
    ("$and", "complete.and"),
    ("$or", "complete.or"),
    ("$nor", "complete.nor"),
    ("$expr", "complete.expr"),
];

fn operators_for(field: Option<&Field>) -> Vec<(&'static str, &'static str)> {
    let has = |names: &[&str]| field.is_some_and(|f| f.types.iter().any(|(t, _)| names.contains(t)));
    let unknown = field.is_none();
    let numeric = has(&["int", "long", "double", "decimal"]);
    let ordered = unknown || numeric || has(&["string", "date", "objectId", "timestamp"]);
    let mut ops = vec![("$eq", "complete.eq"), ("$ne", "complete.ne")];
    if ordered {
        ops.extend([
            ("$gt", "complete.gt"),
            ("$gte", "complete.gte"),
            ("$lt", "complete.lt"),
            ("$lte", "complete.lte"),
        ]);
    }
    ops.extend([
        ("$in", "complete.in"),
        ("$nin", "complete.nin"),
        ("$exists", "complete.exists"),
        ("$type", "complete.type"),
    ]);
    if unknown || has(&["string"]) {
        ops.push(("$regex", "complete.regex"));
    }
    if unknown || has(&["array"]) {
        ops.extend([
            ("$elemMatch", "complete.elem_match"),
            ("$all", "complete.all"),
            ("$size", "complete.size"),
        ]);
    }
    if unknown || numeric {
        ops.push(("$mod", "complete.mod"));
    }
    ops.push(("$not", "complete.not"));
    ops
}

const TYPES: &[&str] = &[
    "string",
    "int",
    "long",
    "double",
    "decimal",
    "number",
    "bool",
    "date",
    "objectId",
    "array",
    "object",
    "null",
    "binData",
    "regex",
    "timestamp",
];

fn describe(field: &Field, sampled: u64) -> String {
    let types: Vec<&str> = field.types.iter().take(3).map(|(t, _)| *t).collect();
    let share = if sampled == 0 {
        100
    } else {
        (field.count * 100).div_ceil(sampled).min(100)
    };
    format!("{} · {share}%", types.join(", "))
}

/// How well `name` matches what was typed: lower is better, None is no match.
fn rank(name: &str, typed: &str) -> Option<u8> {
    if typed.is_empty() {
        return Some(0);
    }
    let (name, typed) = (name.to_lowercase(), typed.to_lowercase());
    if name.starts_with(&typed) {
        Some(0)
    } else if name.split('.').any(|part| part.starts_with(&typed)) {
        Some(1)
    } else if name.contains(&typed) {
        Some(2)
    } else {
        None
    }
}

/// A key as it must be written: dotted paths and odd names need quotes.
fn key_text(name: &str, quoted: bool) -> String {
    let plain = name
        .chars()
        .enumerate()
        .all(|(i, c)| c.is_alphanumeric() || c == '_' || c == '$' && i == 0);
    if quoted || plain {
        name.to_string()
    } else {
        format!("\"{name}\"")
    }
}

pub fn complete(text: &str, cursor: usize, mode: Mode, schema: &Schema) -> Completions {
    let cursor = cursor.min(text.len());
    // The word being typed, which the suggestion replaces.
    let start = text[..cursor]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word_char(c))
        .last()
        .map_or(cursor, |(i, _)| i);
    let end = text[cursor..]
        .char_indices()
        .find(|&(_, c)| !is_word_char(c))
        .map_or(text.len(), |(i, _)| cursor + i);
    let quote = text[..start].chars().last().filter(|c| matches!(c, '"' | '\''));
    let typed = &text[start..cursor];
    let scan_to = if quote.is_some() { start - 1 } else { start };
    let after = &text[end..];
    let after = after.strip_prefix(['"', '\'']).unwrap_or(after);
    let colon = if after.trim_start().starts_with(':') { "" } else { ": " };

    let mut items: Vec<(u8, Suggestion)> = Vec::new();
    let mut push_key = |label: &str, detail: String, kind: Kind, rank: u8| {
        let insert = format!(
            "{}{}",
            key_text(label, quote.is_some()),
            if quote.is_some() { "" } else { colon }
        );
        items.push((
            rank,
            Suggestion {
                label: label.to_string(),
                insert,
                detail,
                kind,
            },
        ));
    };
    match slot(&frames(text, scan_to), mode) {
        Slot::Field { prefix, root } => {
            let mut fields: Vec<&Field> = schema.fields.iter().filter(|f| f.path.starts_with(&prefix)).collect();
            fields.sort_by(|a, b| b.count.cmp(&a.count).then(a.path.cmp(&b.path)));
            for field in fields {
                let name = &field.path[prefix.len()..];
                if let Some(r) = rank(name, typed) {
                    push_key(name, describe(field, schema.sampled), Kind::Field, r);
                }
            }
            if root && mode == Mode::Filter && typed.starts_with('$') {
                for (op, meaning) in LOGICAL {
                    if let Some(r) = rank(op, typed) {
                        push_key(op, meaning.to_string(), Kind::Operator, r);
                    }
                }
            }
        }
        Slot::Operator(path) => {
            for (op, meaning) in operators_for(schema.field(&path)) {
                if let Some(r) = rank(op, typed) {
                    push_key(op, meaning.to_string(), Kind::Operator, r);
                }
            }
        }
        Slot::Value(key) => {
            let values: &[&str] = match (mode, key.as_str()) {
                (Mode::Sort, _) => &["1", "-1"],
                (Mode::Project, _) => &["1", "0"],
                (Mode::Filter, "$exists") => &["true", "false"],
                (Mode::Filter, "$type") => TYPES,
                _ => &[],
            };
            let in_type = key == "$type";
            for value in values {
                if let Some(r) = rank(value, typed) {
                    let insert = if in_type && quote.is_none() {
                        format!("\"{value}\"")
                    } else {
                        value.to_string()
                    };
                    items.push((
                        r,
                        Suggestion {
                            label: value.to_string(),
                            insert,
                            detail: String::new(),
                            kind: Kind::Value,
                        },
                    ));
                }
            }
        }
        Slot::Nothing => {}
    }
    // Stable, so the order within a rank (presence, list order) is kept.
    items.sort_by_key(|(r, _)| *r);
    Completions {
        replace: start..end,
        items: items.into_iter().map(|(_, s)| s).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    fn schema() -> Schema {
        crate::schema::infer(&[
            doc! {
                "name": "a", "age": 30, "tags": ["x"],
                "address": { "city": "Izmir", "zip": 35000 },
                "items": [{ "sku": "s1", "qty": 2 }],
                "createdAt": mongodb::bson::DateTime::now(),
            },
            doc! { "name": "b", "age": 31, "active": true },
        ])
    }

    /// Completes at the `|` in `text`.
    fn at(text: &str, mode: Mode) -> Completions {
        let cursor = text.find('|').expect("cursor marker");
        let text = text.replace('|', "");
        complete(&text, cursor, mode, &schema())
    }

    fn labels(c: &Completions) -> Vec<&str> {
        c.items.iter().map(|s| s.label.as_str()).collect()
    }

    #[test]
    fn suggests_fields_at_the_start_of_a_key_most_common_first() {
        let c = at("{ | }", Mode::Filter);
        assert_eq!(&labels(&c)[..2], ["age", "name"], "in every sampled document");
        assert!(labels(&c).contains(&"address.city"));
        assert!(labels(&c).contains(&"items.sku"));
        assert!(!labels(&c).contains(&"$and"), "operators only when typing $");
        assert_eq!(c.replace, 2..2);
    }

    #[test]
    fn filters_by_what_was_typed_and_replaces_the_word() {
        let c = at("{ ci| }", Mode::Filter);
        assert_eq!(labels(&c), ["address.city"], "matches a later path segment");
        assert_eq!(c.replace, 2..4);
        assert_eq!(c.items[0].insert, "\"address.city\": ", "dotted paths are quoted");
        let c = at("{ na| }", Mode::Filter);
        assert_eq!(c.items[0].insert, "name: ");
        assert_eq!(c.items[0].detail, "string · 100%");
    }

    #[test]
    fn does_not_add_a_colon_when_one_follows() {
        let c = at("{ na|me: 1 }", Mode::Filter);
        assert_eq!(c.replace, 2..6, "the whole word is replaced");
        assert_eq!(c.items[0].insert, "name");
    }

    #[test]
    fn inside_quotes_inserts_the_bare_name() {
        let c = at("{ \"addr|\" }", Mode::Filter);
        assert_eq!(c.items[0].label, "address");
        assert_eq!(c.items[0].insert, "address");
    }

    #[test]
    fn later_keys_after_a_comma_get_fields_too() {
        let c = at("{ age: 5, act| }", Mode::Filter);
        assert_eq!(labels(&c), ["active"]);
        assert_eq!(c.items[0].detail, "bool · 50%");
    }

    #[test]
    fn suggests_operators_that_fit_the_field_type() {
        let ops = labels(&at("{ age: { | } }", Mode::Filter)).join(" ");
        assert!(ops.contains("$gt") && ops.contains("$mod"), "{ops}");
        assert!(!ops.contains("$regex") && !ops.contains("$size"), "{ops}");
        let ops = labels(&at("{ name: { | } }", Mode::Filter)).join(" ");
        assert!(ops.contains("$regex"), "{ops}");
        let ops = labels(&at("{ tags: { | } }", Mode::Filter)).join(" ");
        assert!(ops.contains("$elemMatch") && ops.contains("$size"), "{ops}");
        let ops = labels(&at("{ active: { | } }", Mode::Filter)).join(" ");
        assert!(!ops.contains("$gt"), "booleans aren't ordered: {ops}");
        assert_eq!(at("{ age: { $g| } }", Mode::Filter).items[0].insert, "$gt: ");
    }

    #[test]
    fn logical_operators_go_back_to_fields() {
        assert!(labels(&at("{ $or: [ { | } ] }", Mode::Filter)).contains(&"name"));
        let c = at("{ $| }", Mode::Filter);
        assert_eq!(&labels(&c)[..2], ["$and", "$or"]);
    }

    #[test]
    fn elem_match_suggests_the_array_s_subfields() {
        let c = at("{ items: { $elemMatch: { | } } }", Mode::Filter);
        assert_eq!(labels(&c), ["qty", "sku"]);
    }

    #[test]
    fn values_where_the_choices_are_known() {
        assert_eq!(labels(&at("{ age: | }", Mode::Sort)), ["1", "-1"]);
        assert_eq!(labels(&at("{ age: | }", Mode::Project)), ["1", "0"]);
        assert_eq!(labels(&at("{ age: { $exists: | } }", Mode::Filter)), ["true", "false"]);
        let c = at("{ age: { $type: \"lo|\" } }", Mode::Filter);
        assert_eq!(c.items[0].insert, "long");
        assert!(
            at("{ age: | }", Mode::Filter).items.is_empty(),
            "any value fits a filter"
        );
    }

    #[test]
    fn sort_and_projection_only_take_top_level_fields() {
        let c = at("{ age: 1, | }", Mode::Sort);
        assert!(labels(&c).contains(&"address.city"));
        assert!(at("{ age: { | } }", Mode::Sort).items.is_empty());
    }

    #[test]
    fn strings_and_regexes_do_not_confuse_the_context() {
        for text in [
            "{ name: \"a, b: {\", ag| }",
            "{ name: /x{,]/i, ag| }",
            "{ _id: ObjectId(\"65a1\"), ag| }",
        ] {
            assert_eq!(labels(&at(text, Mode::Filter)).first(), Some(&"age"), "{text}");
        }
    }

    #[test]
    fn nothing_outside_an_object_or_without_a_schema() {
        assert!(at("|", Mode::Filter).items.is_empty());
        assert!(at("{ a: [ | ] }", Mode::Filter).items.is_empty());
        let c = complete("{ }", 2, Mode::Filter, &Schema::default());
        assert!(c.items.is_empty());
    }
}
