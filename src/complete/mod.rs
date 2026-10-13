//! Suggestions for MQL typed into the query boxes and the shell: what fits
//! where the cursor is, given the collection's sampled schema and MongoDB's
//! operator specifications.
//!
//! The text is scanned up to the cursor into its open brackets; walking them
//! from the outside in through [`Ctx`] says what the innermost one holds (a
//! filter, a stage, an expression...), and so what may be typed next.

use std::ops::Range;

pub mod check;
pub mod shell;

use crate::mql::{self, Op};
use crate::schema::{Field, Schema};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Filter,
    Sort,
    Project,
    /// An aggregation pipeline: an array of stages.
    Pipeline,
    /// An update document (`{ $set: ... }`) or an update pipeline.
    Update,
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
    /// Types and presence for a field, the specification's summary for an operator.
    pub detail: String,
    /// A short, translated meaning to show instead of `detail`, when there is one.
    pub detail_key: Option<&'static str>,
    /// The operator's documentation page.
    pub link: Option<&'static str>,
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
    /// Where each element starts, for arrays.
    items: Vec<usize>,
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
                items: if c == '[' { vec![i + 1] } else { Vec::new() },
            }),
            '}' | ']' => {
                stack.pop();
            }
            ':' => {
                if let Some(top) = stack.last_mut().filter(|f| f.bracket == Bracket::Object) {
                    top.expect_key = false;
                }
            }
            ',' => match stack.last_mut() {
                Some(top) if top.bracket == Bracket::Object => {
                    top.expect_key = true;
                    top.key = None;
                }
                Some(top) => top.items.push(i + 1),
                None => {}
            },
            '"' | '\'' | '`' => {
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

/// What a bracket (or a value) holds.
#[derive(Clone, Debug)]
enum Ctx {
    /// A filter; `prefix` is the path inside `$elemMatch`, `root` whether
    /// `$and`-style operators fit.
    Query {
        prefix: String,
        root: bool,
    },
    /// The operators of a field: `{ age: { | } }`.
    FieldOps(String),
    /// Sort keys, valued 1 or -1.
    Sort,
    /// Projection keys, valued 1 or 0.
    Project,
    /// `$project`, `$addFields` and `$set` in a pipeline: fields, valued by
    /// 1/0 or an expression.
    ProjectSpec,
    /// A pipeline stage: `{ $match: ... }`.
    Stage,
    /// `$group`: `_id` and output fields.
    Group,
    /// An output field of `$group`: `{ $sum: ... }`.
    Accumulator,
    /// An expression: operators as keys, field references as values.
    Expr,
    /// A projection value: an expression, or 1/0.
    ProjectValue,
    /// Named arguments of an operator.
    Args(&'static Op),
    /// Update operators: `{ $set: ... }`.
    UpdateOps,
    /// Fields an update operator sets.
    UpdateFields,
    /// `$facet`: a pipeline per output field.
    Facet,
    /// An array of these.
    List(Box<Ctx>),
    /// A value naming a field: `localField: "a"`.
    FieldName,
    Bool,
    TypeName,
    SortValue,
    Unknown,
}

enum Step<'a> {
    Key(&'a str),
    Element,
}

fn pipeline() -> Ctx {
    Ctx::List(Box::new(Ctx::Stage))
}

/// What the value under `step` holds, inside a bracket holding `ctx`.
fn child(ctx: &Ctx, step: Step) -> Ctx {
    use Step::*;
    match (ctx, step) {
        (Ctx::List(inner), Element) => (**inner).clone(),
        (Ctx::Expr | Ctx::ProjectValue, Element) => Ctx::Expr,
        (Ctx::FieldName, Element) => Ctx::FieldName,
        (_, Element) => Ctx::Unknown,
        (Ctx::Query { prefix, root }, Key(k)) => match k {
            "$and" | "$or" | "$nor" => Ctx::List(Box::new(Ctx::Query {
                prefix: prefix.clone(),
                root: *root,
            })),
            "$expr" => Ctx::Expr,
            k if k.starts_with('$') => Ctx::Unknown,
            field => Ctx::FieldOps(format!("{prefix}{field}")),
        },
        (Ctx::FieldOps(field), Key(k)) => match k {
            "$elemMatch" => Ctx::Query {
                prefix: format!("{field}."),
                root: false,
            },
            "$not" => Ctx::FieldOps(field.clone()),
            "$exists" => Ctx::Bool,
            "$type" => Ctx::TypeName,
            _ => Ctx::Unknown,
        },
        (Ctx::Sort, Key(_)) => Ctx::SortValue,
        (Ctx::Project, Key(_)) => Ctx::ProjectValue,
        (Ctx::ProjectSpec, Key(_)) => Ctx::ProjectValue,
        (Ctx::Stage, Key(name)) => match name {
            "$match" => Ctx::Query {
                prefix: String::new(),
                root: true,
            },
            "$sort" => Ctx::Sort,
            "$project" | "$addFields" | "$set" => Ctx::ProjectSpec,
            "$group" => Ctx::Group,
            "$facet" => Ctx::Facet,
            "$unset" => Ctx::FieldName,
            "$replaceWith" | "$sortByCount" | "$redact" => Ctx::Expr,
            name => match mql::find(name, mql::Kind::STAGE) {
                Some(op) if op.encode == mql::Encode::Object => Ctx::Args(op),
                _ => Ctx::Unknown,
            },
        },
        (Ctx::Facet, Key(_)) => pipeline(),
        (Ctx::Group, Key("_id")) => Ctx::Expr,
        (Ctx::Group, Key(_)) => Ctx::Accumulator,
        (Ctx::Accumulator, Key(name)) => match mql::find(name, mql::Kind::ACCUMULATOR) {
            Some(op) if op.encode == mql::Encode::Object => Ctx::Args(op),
            Some(_) => Ctx::Expr,
            None => Ctx::Unknown,
        },
        (Ctx::Expr | Ctx::ProjectValue, Key(k)) => match mql::find(k, mql::Kind::EXPRESSION) {
            Some(op) if op.encode == mql::Encode::Object => Ctx::Args(op),
            _ => Ctx::Expr,
        },
        (Ctx::Args(op), Key(k)) => {
            let Some(arg) = op.args.iter().find(|a| a.name == k) else {
                return Ctx::Unknown;
            };
            let has = |names: &[&str]| arg.types.iter().any(|t| names.contains(t));
            if k == "localField" {
                Ctx::FieldName
            } else if has(&["pipeline"]) {
                // `$lookup`'s pipeline runs on another collection, whose fields aren't known.
                if matches!(op.name, "$lookup" | "$unionWith" | "$graphLookup") {
                    Ctx::Unknown
                } else {
                    pipeline()
                }
            } else if has(&["query"]) {
                Ctx::Query {
                    prefix: String::new(),
                    root: true,
                }
            } else if has(&["sortBy", "sortSpec"]) {
                Ctx::Sort
            } else if has(&["accumulator"]) {
                Ctx::Accumulator
            } else if arg
                .types
                .iter()
                .any(|t| *t == "expression" || t.starts_with("resolvesTo") || *t == "arrayFieldPath")
            {
                Ctx::Expr
            } else {
                Ctx::Unknown
            }
        }
        (Ctx::UpdateOps, Key(k)) if k.starts_with('$') => Ctx::UpdateFields,
        _ => Ctx::Unknown,
    }
}

/// The context of the outermost bracket.
fn root(mode: Mode, bracket: Bracket) -> Ctx {
    let query = Ctx::Query {
        prefix: String::new(),
        root: true,
    };
    match (mode, bracket) {
        (Mode::Filter, Bracket::Object) => query,
        (Mode::Sort, Bracket::Object) => Ctx::Sort,
        (Mode::Project, Bracket::Object) => Ctx::Project,
        (Mode::Pipeline | Mode::Update, Bracket::Array) => pipeline(),
        (Mode::Pipeline, Bracket::Object) => Ctx::Stage,
        (Mode::Update, Bracket::Object) => Ctx::UpdateOps,
        _ => Ctx::Unknown,
    }
}

/// Short, translated meanings for the operators people use most.
const SHORT: &[(&str, &str)] = &[
    ("$and", "complete.and"),
    ("$or", "complete.or"),
    ("$nor", "complete.nor"),
    ("$expr", "complete.expr"),
    ("$eq", "complete.eq"),
    ("$ne", "complete.ne"),
    ("$gt", "complete.gt"),
    ("$gte", "complete.gte"),
    ("$lt", "complete.lt"),
    ("$lte", "complete.lte"),
    ("$in", "complete.in"),
    ("$nin", "complete.nin"),
    ("$exists", "complete.exists"),
    ("$type", "complete.type"),
    ("$regex", "complete.regex"),
    ("$elemMatch", "complete.elem_match"),
    ("$all", "complete.all"),
    ("$size", "complete.size"),
    ("$mod", "complete.mod"),
    ("$not", "complete.not"),
];

/// Field operators in the order people reach for them; the rest follow alphabetically.
const FIELD_OP_ORDER: &[&str] = &[
    "$eq",
    "$ne",
    "$gt",
    "$gte",
    "$lt",
    "$lte",
    "$in",
    "$nin",
    "$exists",
    "$type",
    "$regex",
    "$elemMatch",
    "$all",
    "$size",
    "$not",
    "$mod",
];
const QUERY_OP_ORDER: &[&str] = &["$and", "$or", "$nor", "$expr"];
const STAGE_ORDER: &[&str] = &[
    "$match",
    "$project",
    "$group",
    "$sort",
    "$limit",
    "$skip",
    "$unwind",
    "$lookup",
    "$addFields",
    "$set",
    "$count",
];

fn ordered<'a>(ops: impl Iterator<Item = &'a Op>, first: &[&str]) -> Vec<&'a Op> {
    let mut ops: Vec<&Op> = ops.collect();
    ops.sort_by_key(|op| first.iter().position(|n| *n == op.name).unwrap_or(first.len()));
    ops
}

/// Whether a field operator makes sense for a field with these types.
fn fits(op: &str, field: Option<&Field>) -> bool {
    let Some(field) = field else { return true };
    let has = |names: &[&str]| field.types.iter().any(|(t, _)| names.contains(t));
    let numeric = has(&["int", "long", "double", "decimal"]);
    match op {
        "$gt" | "$gte" | "$lt" | "$lte" => numeric || has(&["string", "date", "objectId", "timestamp"]),
        "$regex" => has(&["string"]),
        "$elemMatch" | "$all" | "$size" => has(&["array"]),
        "$mod" => numeric,
        o if o.starts_with("$bits") => numeric || has(&["binData"]),
        "$geoWithin" | "$geoIntersects" | "$near" | "$nearSphere" => has(&["object", "array"]),
        _ => true,
    }
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

/// The schema at the stage being typed: the collection's, run through the
/// complete stages before it.
fn schema_at(text: &str, stack: &[Frame], mode: Mode, schema: &Schema) -> Schema {
    let Some(first) = stack.first().filter(|f| f.bracket == Bracket::Array) else {
        return schema.clone();
    };
    if !matches!(mode, Mode::Pipeline | Mode::Update) {
        return schema.clone();
    }
    let mut stages = Vec::new();
    for pair in first.items.windows(2) {
        let src = text[pair[0]..pair[1] - 1].trim();
        match crate::query::parse_document(src) {
            Ok(stage) => stages.push(stage),
            Err(_) => break,
        }
    }
    schema.after(&stages)
}

struct Builder<'a> {
    typed: &'a str,
    quote: Option<char>,
    colon: &'static str,
    items: Vec<(u8, Suggestion)>,
}

impl Builder<'_> {
    fn key(&mut self, label: &str, detail: String, detail_key: Option<&'static str>, kind: Kind) {
        self.key_with_link(label, detail, detail_key, None, kind);
    }

    fn key_with_link(
        &mut self,
        label: &str,
        detail: String,
        detail_key: Option<&'static str>,
        link: Option<&'static str>,
        kind: Kind,
    ) {
        let Some(r) = rank(label, self.typed) else { return };
        let quoted = self.quote.is_some();
        let insert = format!("{}{}", key_text(label, quoted), if quoted { "" } else { self.colon });
        self.items.push((
            r,
            Suggestion {
                label: label.to_string(),
                insert,
                detail,
                detail_key,
                link,
                kind,
            },
        ));
    }

    fn op(&mut self, op: &Op) {
        let short = SHORT.iter().find(|(n, _)| *n == op.name).map(|(_, k)| *k);
        self.key_with_link(op.name, op.summary.to_string(), short, Some(op.link), Kind::Operator);
    }

    fn fields(&mut self, schema: &Schema, prefix: &str) {
        let mut fields: Vec<&Field> = schema.fields.iter().filter(|f| f.path.starts_with(prefix)).collect();
        fields.sort_by(|a, b| b.count.cmp(&a.count).then(a.path.cmp(&b.path)));
        for field in fields {
            self.key(
                &field.path[prefix.len()..],
                describe(field, schema.sampled),
                None,
                Kind::Field,
            );
        }
    }

    /// A value: quoted when `string` and not already inside quotes.
    fn value(&mut self, label: &str, detail: String, string: bool, kind: Kind) {
        let Some(r) = rank(label, self.typed) else { return };
        let insert = if string && self.quote.is_none() {
            format!("\"{label}\"")
        } else {
            label.to_string()
        };
        self.items.push((
            r,
            Suggestion {
                label: label.to_string(),
                insert,
                detail,
                detail_key: None,
                link: None,
                kind,
            },
        ));
    }

    /// `"$field"` references, once a `$` is typed.
    fn references(&mut self, schema: &Schema) {
        if !self.typed.starts_with('$') {
            return;
        }
        let mut fields: Vec<&Field> = schema.fields.iter().collect();
        fields.sort_by(|a, b| b.count.cmp(&a.count).then(a.path.cmp(&b.path)));
        for field in fields {
            self.value(
                &format!("${}", field.path),
                describe(field, schema.sampled),
                true,
                Kind::Field,
            );
        }
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
    let quote = text[..start].chars().last().filter(|c| matches!(c, '"' | '\'' | '`'));
    let scan_to = if quote.is_some() { start - 1 } else { start };
    let after = &text[end..];
    let after = after.strip_prefix(['"', '\'', '`']).unwrap_or(after);
    let colon = if after.trim_start().starts_with(':') { "" } else { ": " };
    let mut b = Builder {
        typed: &text[start..cursor],
        quote,
        colon,
        items: Vec::new(),
    };

    let stack = frames(text, scan_to);
    if let Some(first) = stack.first() {
        let schema = schema_at(text, &stack, mode, schema);
        let mut ctx = root(mode, first.bracket);
        for frame in &stack[..stack.len() - 1] {
            let step = match frame.bracket {
                Bracket::Object => Step::Key(frame.key.as_deref().unwrap_or("")),
                Bracket::Array => Step::Element,
            };
            ctx = child(&ctx, step);
        }
        let top = stack.last().expect("not empty");
        match top.bracket {
            Bracket::Object if top.expect_key => keys(&mut b, &ctx, &schema),
            Bracket::Object => {
                let value = child(&ctx, Step::Key(top.key.as_deref().unwrap_or("")));
                values(&mut b, &value, &schema);
            }
            Bracket::Array => values(&mut b, &child(&ctx, Step::Element), &schema),
        }
    }
    let mut items = b.items;
    // Stable, so the order within a rank (presence, list order) is kept.
    items.sort_by_key(|(r, _)| *r);
    Completions {
        replace: start..end,
        items: items.into_iter().map(|(_, s)| s).collect(),
    }
}

/// What may be typed as a key in a bracket holding `ctx`.
fn keys(b: &mut Builder, ctx: &Ctx, schema: &Schema) {
    match ctx {
        Ctx::Query { prefix, root } => {
            b.fields(schema, prefix);
            if *root && b.typed.starts_with('$') {
                for op in ordered(mql::of(mql::Kind::QUERY), QUERY_OP_ORDER) {
                    b.op(op);
                }
            }
        }
        Ctx::FieldOps(path) => {
            let field = schema.field(path);
            for op in ordered(mql::of(mql::Kind::FIELD_QUERY), FIELD_OP_ORDER) {
                if fits(op.name, field) {
                    b.op(op);
                }
            }
        }
        Ctx::Sort | Ctx::Project | Ctx::ProjectSpec | Ctx::UpdateFields => b.fields(schema, ""),
        Ctx::Stage => {
            for op in ordered(mql::of(mql::Kind::STAGE), STAGE_ORDER) {
                b.op(op);
            }
        }
        Ctx::Group => b.key("_id", String::new(), None, Kind::Field),
        Ctx::Accumulator => {
            for op in mql::of(mql::Kind::ACCUMULATOR) {
                b.op(op);
            }
        }
        Ctx::Expr | Ctx::ProjectValue if b.typed.starts_with('$') => {
            for op in mql::of(mql::Kind::EXPRESSION) {
                b.op(op);
            }
        }
        Ctx::Args(op) => {
            for arg in op.args {
                b.key(arg.name, arg.types.join(" | "), None, Kind::Field);
            }
        }
        Ctx::UpdateOps => {
            for op in mql::of(mql::Kind::UPDATE) {
                b.op(op);
            }
        }
        _ => {}
    }
}

/// What may be typed as a value holding `ctx`.
fn values(b: &mut Builder, ctx: &Ctx, schema: &Schema) {
    let plain = |b: &mut Builder, values: &[&str], string: bool| {
        for v in values {
            b.value(v, String::new(), string, Kind::Value);
        }
    };
    match ctx {
        Ctx::SortValue => plain(b, &["1", "-1"], false),
        Ctx::ProjectValue => {
            plain(b, &["1", "0"], false);
            b.references(schema);
        }
        Ctx::Bool => plain(b, &["true", "false"], false),
        Ctx::TypeName => plain(b, TYPES, true),
        Ctx::Expr | Ctx::Accumulator => b.references(schema),
        Ctx::FieldName => {
            let mut fields: Vec<&Field> = schema.fields.iter().collect();
            fields.sort_by(|a, b| b.count.cmp(&a.count).then(a.path.cmp(&b.path)));
            for field in fields {
                b.value(&field.path, describe(field, schema.sampled), true, Kind::Field);
            }
        }
        // `$unwind: "$tags"`: the short form takes the first argument.
        Ctx::Args(op)
            if op
                .args
                .first()
                .is_some_and(|a| a.types.iter().any(|t| *t == "arrayFieldPath" || *t == "expression")) =>
        {
            b.references(schema);
        }
        _ => {}
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

    #[test]
    fn operators_come_from_the_specification() {
        let c = at("{ age: { $| } }", Mode::Filter);
        let gt = c.items.iter().find(|s| s.label == "$gt").unwrap();
        assert_eq!(
            gt.detail_key,
            Some("complete.gt"),
            "short translated meaning where we have one"
        );
        assert!(gt.detail.starts_with("Matches values"), "{}", gt.detail);
        assert!(labels(&c).contains(&"$bitsAllSet"), "operators beyond our short list");
        assert!(!labels(&at("{ name: { $| } }", Mode::Filter)).contains(&"$bitsAllSet"));
    }

    #[test]
    fn pipeline_stages_and_their_contents() {
        let c = at("[ { $| } ]", Mode::Pipeline);
        assert_eq!(&labels(&c)[..3], ["$match", "$project", "$group"]);
        assert!(labels(&c).contains(&"$bucketAuto"));
        assert_eq!(c.items[0].insert, "$match: ");
        assert_eq!(labels(&at("[ { $match: { ag| } } ]", Mode::Pipeline))[0], "age");
        assert_eq!(labels(&at("[ { $sort: { age: | } } ]", Mode::Pipeline)), ["1", "-1"]);
        assert!(labels(&at("[{ $match: { age: { $| } } }]", Mode::Pipeline)).contains(&"$gte"));
    }

    #[test]
    fn group_keys_accumulators_and_references() {
        assert_eq!(labels(&at("[ { $group: { _| } } ]", Mode::Pipeline)), ["_id"]);
        let c_acc = at("[ { $group: { _id: null, n: { $s| } } } ]", Mode::Pipeline);
        let acc = labels(&c_acc);
        assert!(acc.contains(&"$sum") && acc.contains(&"$stdDevPop"), "{acc:?}");
        let c = at("[ { $group: { _id: \"$addr|\" } } ]", Mode::Pipeline);
        assert_eq!(&labels(&c)[..3], ["$address", "$address.city", "$address.zip"]);
        assert_eq!(c.items[0].insert, "$address", "already inside quotes");
        let c = at("[ { $group: { _id: $na| } } ]", Mode::Pipeline);
        assert_eq!(c.items[0].insert, "\"$name\"", "quoted when typed bare");
    }

    #[test]
    fn expressions_suggest_operators_and_references() {
        let c_ops = at("[ { $project: { up: { $toU| } } } ]", Mode::Pipeline);
        let ops = labels(&c_ops);
        assert_eq!(ops, ["$toUpper"]);
        let c_refs = at("[ { $project: { up: { $toUpper: \"$n|\" } } } ]", Mode::Pipeline);
        let refs = labels(&c_refs);
        assert_eq!(refs.first(), Some(&"$name"));
        let c_refs = at(
            "[ { $project: { s: { $concat: [\"$name\", \"$a|\"] } } } ]",
            Mode::Pipeline,
        );
        let refs = labels(&c_refs);
        assert!(refs.contains(&"$age") && refs.contains(&"$address.city"), "{refs:?}");
        assert_eq!(
            labels(&at("[ { $project: { name: | } } ]", Mode::Pipeline))[..2],
            ["1", "0"]
        );
    }

    #[test]
    fn object_stages_suggest_their_arguments() {
        let c_args = at("[ { $lookup: { | } } ]", Mode::Pipeline);
        let args = labels(&c_args);
        for name in ["from", "localField", "foreignField", "as", "pipeline"] {
            assert!(args.contains(&name), "{args:?}");
        }
        assert_eq!(
            labels(&at("[ { $lookup: { localField: \"ad|\" } } ]", Mode::Pipeline))[0],
            "address"
        );
        assert_eq!(
            labels(&at("[ { $unwind: { path: \"$ta|\" } } ]", Mode::Pipeline)),
            ["$tags"]
        );
        assert_eq!(labels(&at("[ { $unwind: \"$it|\" } ]", Mode::Pipeline))[0], "$items");
    }

    #[test]
    fn later_stages_see_the_reshaped_fields() {
        let c = at(
            "[ { $group: { _id: \"$address.city\", n: { $sum: 1 } } }, { $match: { | } } ]",
            Mode::Pipeline,
        );
        assert_eq!(labels(&c), ["_id", "n"]);
        assert_eq!(c.items[1].detail, "int · 100%");
        let c = at(
            "[ { $unwind: \"$items\" }, { $project: { sku: \"$items.s|\" } } ]",
            Mode::Pipeline,
        );
        assert_eq!(labels(&c), ["$items.sku"]);
        let c = at("[ { $project: { name: 1 } }, { $sort: { | } } ]", Mode::Pipeline);
        assert_eq!(labels(&c), ["name"], "no _id in this sample");
        let c = at("[ { $match: { age: { $gt: } } }, { $sort: { | } } ]", Mode::Pipeline);
        assert!(
            labels(&c).contains(&"age"),
            "a stage that doesn't parse stops the reshaping, not completion"
        );
    }

    #[test]
    fn facet_holds_pipelines() {
        assert_eq!(
            labels(&at("[ { $facet: { a: [ { $li| } ] } } ]", Mode::Pipeline))[0],
            "$limit"
        );
    }

    #[test]
    fn update_operators_then_fields() {
        let c_ops = at("{ $| }", Mode::Update);
        let ops = labels(&c_ops);
        assert!(
            ops.contains(&"$set") && ops.contains(&"$inc") && ops.contains(&"$push"),
            "{ops:?}"
        );
        assert_eq!(labels(&at("{ $set: { ag| } }", Mode::Update))[0], "age");
        assert!(
            labels(&at("[ { $| } ]", Mode::Update)).contains(&"$set"),
            "update pipelines"
        );
    }
}
