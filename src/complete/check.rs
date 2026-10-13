//! Warnings for MQL as it is typed: fields that aren't in the sampled
//! documents, unknown operators, values of the wrong type for a field, and
//! operator arguments of the wrong shape (from the specifications).
//!
//! The text is read by a forgiving parser that keeps whatever parses before
//! a syntax error, and walked with the same contexts as completion.

use std::ops::Range;

use rust_i18n::t;

use super::{Ctx, Mode, Step, child, root};
use crate::mql::{self, Encode};
use crate::schema::Schema;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Message {
    /// `everywhere`: the whole collection was checked, not just a sample.
    UnknownField {
        field: String,
        suggestion: Option<String>,
        everywhere: bool,
    },
    UnknownOperator {
        op: String,
        suggestion: Option<String>,
    },
    UnknownArgument {
        op: String,
        arg: String,
        suggestion: Option<String>,
    },
    MissingArgument {
        op: String,
        arg: String,
    },
    TypeMismatch {
        field: String,
        types: String,
        value: String,
    },
    ArgumentType {
        op: String,
        types: String,
    },
    ArgumentCount {
        op: String,
        count: usize,
    },
    OneOperatorPerStage,
}

impl Message {
    pub fn text(&self) -> String {
        let hint = |s: &Option<String>| match s {
            Some(name) => format!(" {}", t!("check.did_you_mean", name = name)),
            None => String::new(),
        };
        match self {
            Message::UnknownField {
                field,
                suggestion,
                everywhere,
            } => {
                let text = if *everywhere {
                    t!("check.unknown_field_anywhere", field = field)
                } else {
                    t!("check.unknown_field", field = field)
                };
                format!("{text}{}", hint(suggestion))
            }
            Message::UnknownOperator { op, suggestion } => {
                format!("{}{}", t!("check.unknown_operator", op = op), hint(suggestion))
            }
            Message::UnknownArgument { op, arg, suggestion } => {
                format!(
                    "{}{}",
                    t!("check.unknown_argument", op = op, arg = arg),
                    hint(suggestion)
                )
            }
            Message::MissingArgument { op, arg } => t!("check.missing_argument", op = op, arg = arg).into(),
            Message::TypeMismatch { field, types, value } => {
                t!("check.type_mismatch", field = field, types = types, value = value).into()
            }
            Message::ArgumentType { op, types } => t!("check.argument_type", op = op, types = types).into(),
            Message::ArgumentCount { op, count } => t!("check.argument_count", op = op, count = count).into(),
            Message::OneOperatorPerStage => t!("check.one_operator_per_stage").into(),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Problem {
    pub range: Range<usize>,
    pub severity: Severity,
    pub message: Message,
}

// --- A forgiving parser with byte ranges ---

#[derive(Debug)]
enum Value {
    /// A string, with its contents.
    Str(String),
    /// Whether it is non-zero, which in a projection means "include".
    Number(bool),
    Bool(bool),
    Null,
    Regex,
    /// A helper call such as `ObjectId(...)` or a bare identifier.
    Call(String),
}

#[derive(Debug)]
enum Node {
    Object(Vec<Entry>),
    Array(Vec<Spanned>),
    Scalar(Value),
}

#[derive(Debug)]
struct Spanned {
    node: Node,
    range: Range<usize>,
}

#[derive(Debug)]
struct Entry {
    key: String,
    key_range: Range<usize>,
    value: Option<Spanned>,
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn ws(&mut self) {
        loop {
            let rest = &self.src[self.pos..];
            let trimmed = rest.trim_start();
            self.pos += rest.len() - trimmed.len();
            if trimmed.starts_with("//") {
                self.pos += trimmed.find('\n').unwrap_or(trimmed.len());
            } else {
                break;
            }
        }
    }

    fn word(&mut self) -> &str {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '.'))
        {
            self.bump();
        }
        &self.src[start..self.pos]
    }

    fn string(&mut self) -> String {
        let quote = self.bump().unwrap_or('"');
        let mut out = String::new();
        let mut escaped = false;
        while let Some(c) = self.bump() {
            if escaped {
                out.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == quote {
                break;
            } else {
                out.push(c);
            }
        }
        out
    }

    fn value(&mut self) -> Option<Spanned> {
        self.ws();
        let start = self.pos;
        let node = match self.peek()? {
            '{' => {
                self.bump();
                let mut entries = Vec::new();
                loop {
                    self.ws();
                    match self.peek() {
                        Some('}') => {
                            self.bump();
                            break;
                        }
                        Some(',') => {
                            self.bump();
                            continue;
                        }
                        None => break,
                        _ => {}
                    }
                    let key_start = self.pos;
                    let key = match self.peek() {
                        Some('"' | '\'') => self.string(),
                        _ => self.word().to_string(),
                    };
                    let key_range = key_start..self.pos;
                    if key.is_empty() {
                        break;
                    }
                    self.ws();
                    if self.peek() != Some(':') {
                        entries.push(Entry {
                            key,
                            key_range,
                            value: None,
                        });
                        break;
                    }
                    self.bump();
                    let value = self.value();
                    let done = value.is_none();
                    entries.push(Entry { key, key_range, value });
                    if done {
                        break;
                    }
                }
                Node::Object(entries)
            }
            '[' => {
                self.bump();
                let mut items = Vec::new();
                loop {
                    self.ws();
                    match self.peek() {
                        Some(']') => {
                            self.bump();
                            break;
                        }
                        Some(',') => {
                            self.bump();
                            continue;
                        }
                        None => break,
                        _ => {}
                    }
                    match self.value() {
                        Some(v) => items.push(v),
                        None => break,
                    }
                }
                Node::Array(items)
            }
            '"' | '\'' => Node::Scalar(Value::Str(self.string())),
            '/' => {
                self.bump();
                let mut escaped = false;
                while let Some(c) = self.bump() {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '/' {
                        break;
                    }
                }
                self.word();
                Node::Scalar(Value::Regex)
            }
            c if c.is_ascii_digit() || matches!(c, '-' | '+' | '.') => {
                self.bump();
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
                {
                    self.bump();
                }
                let text = &self.src[start..self.pos];
                Node::Scalar(Value::Number(text.parse::<f64>() != Ok(0.0)))
            }
            c if c.is_alphabetic() || c == '_' || c == '$' => {
                let mut name = self.word().to_string();
                if name == "new" {
                    self.ws();
                    name = self.word().to_string();
                }
                self.ws();
                if self.peek() == Some('(') {
                    // Skip the call's arguments, minding nesting and strings.
                    let mut depth = 0;
                    while let Some(c) = self.peek() {
                        match c {
                            '"' | '\'' => {
                                self.string();
                                continue;
                            }
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    self.bump();
                                    break;
                                }
                            }
                            _ => {}
                        }
                        self.bump();
                    }
                }
                match name.as_str() {
                    "true" | "false" => Node::Scalar(Value::Bool(name == "true")),
                    "null" | "undefined" => Node::Scalar(Value::Null),
                    "Infinity" | "NaN" => Node::Scalar(Value::Number(true)),
                    _ => Node::Scalar(Value::Call(name)),
                }
            }
            _ => return None,
        };
        Some(Spanned {
            node,
            range: start..self.pos,
        })
    }
}

fn parse(src: &str) -> Option<Spanned> {
    Parser { src, pos: 0 }.value()
}

// --- Checks ---

/// The family a literal belongs to, in `$type` names.
fn family(value: &Value) -> Option<&'static str> {
    Some(match value {
        Value::Str(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "bool",
        Value::Call(name) => match name.as_str() {
            "ObjectId" | "ObjectID" => "objectId",
            "ISODate" | "Date" => "date",
            "NumberInt" | "NumberLong" | "NumberDecimal" | "Int32" | "Int64" | "Long" | "Decimal128" => "number",
            "UUID" | "BinData" => "binData",
            "Timestamp" => "timestamp",
            _ => return None,
        },
        Value::Null | Value::Regex => return None,
    })
}

fn same_family(field_type: &str, family: &str) -> bool {
    match family {
        "number" => matches!(field_type, "int" | "long" | "double" | "decimal"),
        f => field_type == f,
    }
}

/// Edit distance where swapping two neighbours counts as one edit, the
/// most common typo.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// The closest name, if it is close enough to be a likely typo.
fn closest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    let limit = (name.chars().count() / 3).clamp(1, 3);
    candidates
        .map(|c| (distance(&name.to_lowercase(), &c.to_lowercase()), c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c.to_string())
}

/// The operator family `$` keys belong to in a context.
fn operator_kinds(ctx: &Ctx) -> Option<mql::Kind> {
    Some(match ctx {
        Ctx::Query { root: true, .. } => mql::Kind::QUERY,
        Ctx::Query { root: false, .. } => mql::Kind(mql::Kind::QUERY.0 | mql::Kind::FIELD_QUERY.0),
        Ctx::FieldOps(_) => mql::Kind::FIELD_QUERY,
        Ctx::Stage => mql::Kind::STAGE,
        Ctx::Accumulator => mql::Kind::ACCUMULATOR,
        Ctx::Expr | Ctx::ProjectValue => mql::Kind::EXPRESSION,
        Ctx::UpdateOps => mql::Kind::UPDATE,
        _ => return None,
    })
}

fn is_inclusion(value: &Option<Spanned>) -> bool {
    matches!(
        value,
        Some(Spanned {
            node: Node::Scalar(Value::Number(true) | Value::Bool(true)),
            ..
        })
    )
}

struct Checker<'a> {
    src: &'a str,
    out: Vec<Problem>,
}

impl Checker<'_> {
    fn warn(&mut self, range: Range<usize>, message: Message) {
        self.push(range, Severity::Warning, message);
    }

    fn error(&mut self, range: Range<usize>, message: Message) {
        self.push(range, Severity::Error, message);
    }

    fn push(&mut self, range: Range<usize>, severity: Severity, message: Message) {
        self.out.push(Problem {
            range,
            severity,
            message,
        });
    }

    /// A field path that isn't in the sample, when the sample can tell.
    fn check_field(&mut self, path: &str, range: Range<usize>, schema: &Schema, prefix: &str) {
        if schema.sampled == 0 || schema.truncated || path.split('.').any(|p| p.parse::<usize>().is_ok()) {
            return;
        }
        let under = format!("{path}.");
        if schema.field(path).is_some() || schema.fields.iter().any(|f| f.path.starts_with(&under)) {
            return;
        }
        let suggestion = closest(
            &path[prefix.len()..],
            schema.fields.iter().filter_map(|f| f.path.strip_prefix(prefix)),
        );
        self.warn(
            range,
            Message::UnknownField {
                field: path[prefix.len()..].to_string(),
                suggestion,
                everywhere: schema.complete,
            },
        );
    }

    /// A literal compared with a field that, in the sample, never holds that kind of value.
    fn check_type(&mut self, path: &str, value: &Spanned, schema: &Schema) {
        let Node::Scalar(literal) = &value.node else { return };
        let (Some(family), Some(field)) = (family(literal), schema.field(path)) else {
            return;
        };
        let mut types: Vec<&str> = field
            .types
            .iter()
            .chain(&field.items)
            .map(|(t, _)| *t)
            .filter(|t| !matches!(*t, "null" | "undefined" | "array"))
            .collect();
        types.dedup();
        if types.is_empty() || types.iter().any(|t| same_family(t, family)) {
            return;
        }
        self.warn(
            value.range.clone(),
            Message::TypeMismatch {
                field: path.to_string(),
                types: types.join(", "),
                value: family.to_string(),
            },
        );
    }

    /// The argument of a field operator, against its specification.
    fn check_argument(&mut self, op: &str, value: &Spanned) {
        let Some(spec) = mql::find(op, mql::Kind::FIELD_QUERY) else {
            return;
        };
        match (spec.encode, &value.node) {
            (Encode::Array, Node::Array(items)) => {
                let required = spec.args.iter().filter(|a| !a.optional).count();
                if items.len() < required || items.len() > spec.args.len() {
                    let count = spec.args.len();
                    self.error(value.range.clone(), Message::ArgumentCount { op: op.into(), count });
                }
            }
            (Encode::Array, _) => {
                let count = spec.args.len();
                self.error(value.range.clone(), Message::ArgumentCount { op: op.into(), count });
            }
            (Encode::Single, node) if spec.args.len() == 1 => {
                let arg = &spec.args[0];
                // Only plain types can be checked; "any", "expression"... accept anything.
                let fits = |family: &str| {
                    arg.types
                        .iter()
                        .all(|t| matches!(*t, "int" | "number" | "string" | "bool" | "regex" | "array"))
                        && !arg.types.iter().any(|t| match *t {
                            "int" | "number" => family == "number",
                            "bool" => family == "bool" || family == "number",
                            "regex" => family == "regex" || family == "string",
                            t => t == family,
                        })
                };
                let family = match node {
                    Node::Array(_) => "array",
                    Node::Object(_) => "object",
                    Node::Scalar(Value::Regex) => "regex",
                    Node::Scalar(v) => match family(v) {
                        Some(f) => f,
                        None => return,
                    },
                };
                if arg.variadic.is_some() && family == "array" {
                    return;
                }
                if fits(family) {
                    let types = arg.types.join(" | ");
                    self.error(value.range.clone(), Message::ArgumentType { op: op.into(), types });
                }
            }
            _ => {}
        }
    }

    fn walk(&mut self, node: &Spanned, ctx: &Ctx, schema: &Schema) {
        match &node.node {
            Node::Object(entries) => {
                if matches!(ctx, Ctx::Stage) && entries.len() > 1 {
                    self.error(entries[1].key_range.clone(), Message::OneOperatorPerStage);
                }
                if let Ctx::Args(op) = ctx {
                    for arg in op.args.iter().filter(|a| !a.optional) {
                        if !entries.iter().any(|e| e.key == arg.name) && entries.iter().all(|e| e.value.is_some()) {
                            let range = node.range.start..node.range.start + 1;
                            self.warn(
                                range,
                                Message::MissingArgument {
                                    op: op.name.into(),
                                    arg: arg.name.into(),
                                },
                            );
                        }
                    }
                }
                for entry in entries {
                    self.entry(entry, ctx, schema);
                }
            }
            Node::Array(items) => {
                let inner = child(ctx, Step::Element);
                for item in items {
                    self.walk(item, &inner, schema);
                }
            }
            Node::Scalar(Value::Str(s)) => {
                let refs = matches!(ctx, Ctx::Expr | Ctx::ProjectValue | Ctx::Accumulator);
                if refs && s.starts_with('$') && !s.starts_with("$$") && s.len() > 1 {
                    let range = node.range.start + 1..node.range.end.saturating_sub(1).max(node.range.start + 1);
                    self.check_field(&s[1..], range, schema, "");
                }
            }
            Node::Scalar(_) => {}
        }
    }

    fn entry(&mut self, entry: &Entry, ctx: &Ctx, schema: &Schema) {
        let key = entry.key.as_str();
        let range = entry.key_range.clone();
        if key.starts_with('$') {
            if let Some(kinds) = operator_kinds(ctx)
                && mql::find(key, kinds).is_none()
            {
                let suggestion = closest(key, mql::of(kinds).map(|op| op.name));
                self.error(
                    range,
                    Message::UnknownOperator {
                        op: key.into(),
                        suggestion,
                    },
                );
                return;
            }
        } else {
            match ctx {
                Ctx::Query { prefix, .. } => self.check_field(&format!("{prefix}{key}"), range, schema, prefix),
                Ctx::Sort => self.check_field(key, range, schema, ""),
                Ctx::Project | Ctx::ProjectSpec if is_inclusion(&entry.value) => {
                    self.check_field(key, range, schema, "")
                }
                Ctx::Args(op) if !op.args.iter().any(|a| a.name == key) => {
                    let suggestion = closest(key, op.args.iter().map(|a| a.name));
                    self.error(
                        range,
                        Message::UnknownArgument {
                            op: op.name.into(),
                            arg: key.into(),
                            suggestion,
                        },
                    );
                    return;
                }
                _ => {}
            }
        }
        let Some(value) = &entry.value else { return };
        match ctx {
            Ctx::Query { prefix, .. } if !key.starts_with('$') => {
                self.check_type(&format!("{prefix}{key}"), value, schema)
            }
            Ctx::FieldOps(field) => {
                self.check_argument(key, value);
                match (key, &value.node) {
                    ("$eq" | "$ne" | "$gt" | "$gte" | "$lt" | "$lte", _) => self.check_type(field, value, schema),
                    ("$in" | "$nin", Node::Array(items)) => {
                        for item in items {
                            self.check_type(field, item, schema);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        let _ = self.src;
        self.walk(value, &child(ctx, Step::Key(key)), schema);
    }
}

/// Problems in `text` read as `mode`, against `schema`.
pub fn check(text: &str, mode: Mode, schema: &Schema) -> Vec<Problem> {
    let Some(tree) = parse(text) else {
        return Vec::new();
    };
    let mut checker = Checker {
        src: text,
        out: Vec::new(),
    };
    match (&tree.node, mode) {
        // Each stage sees the fields the stages before it leave.
        (Node::Array(stages), Mode::Pipeline | Mode::Update) => {
            let mut schema = schema.clone();
            for stage in stages {
                checker.walk(stage, &Ctx::Stage, &schema);
                match crate::query::parse_document(&text[stage.range.clone()]) {
                    Ok(doc) => schema = schema.after(std::slice::from_ref(&doc)),
                    Err(_) => break,
                }
            }
        }
        (Node::Object(_), _) => {
            let ctx = root(mode, super::Bracket::Object);
            checker.walk(&tree, &ctx, schema);
        }
        _ => {}
    }
    checker.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    fn schema() -> Schema {
        crate::schema::infer(&[
            doc! {
                "name": "a", "age": 30, "email": "a@b", "tags": ["x"],
                "address": { "city": "Izmir" },
                "items": [{ "sku": "s1", "qty": 2 }],
                "createdAt": mongodb::bson::DateTime::now(),
            },
            doc! { "name": "b", "age": 31, "active": true },
        ])
    }

    fn problems(text: &str, mode: Mode) -> Vec<(String, Message)> {
        check(text, mode, &schema())
            .into_iter()
            .map(|p| (text[p.range].to_string(), p.message))
            .collect()
    }

    fn unknown_field(field: &str, suggestion: Option<&str>) -> Message {
        Message::UnknownField {
            field: field.into(),
            suggestion: suggestion.map(String::from),
            everywhere: false,
        }
    }

    #[test]
    fn a_clean_query_has_no_problems() {
        let text = "{ name: 'a', age: { $gt: 5, $in: [1, 2] }, 'address.city': /iz/i, tags: 'x', \
                    createdAt: { $gte: ISODate('2024-01-01') }, $or: [{ active: true }] }";
        assert_eq!(problems(text, Mode::Filter), []);
    }

    #[test]
    fn unknown_fields_with_a_guess() {
        assert_eq!(
            problems("{ emial: 1 }", Mode::Filter),
            [("emial".into(), unknown_field("emial", Some("email")))]
        );
        assert_eq!(
            problems("{ zzz: 1 }", Mode::Filter),
            [("zzz".into(), unknown_field("zzz", None))]
        );
        assert_eq!(
            problems("{ 'address.cty': 1 }", Mode::Filter)[0].1,
            unknown_field("address.cty", Some("address.city"))
        );
        assert_eq!(
            problems("{ 'tags.0': 'x', address: {} }", Mode::Filter),
            [],
            "indexes and parents are fine"
        );
        assert_eq!(
            problems("{ items: { $elemMatch: { skuu: 1 } } }", Mode::Filter)[0].1,
            unknown_field("skuu", Some("sku"))
        );
        assert_eq!(
            problems("{ agee: 1 }", Mode::Sort)[0].1,
            unknown_field("agee", Some("age"))
        );
        assert_eq!(
            problems("{ agee: 0 }", Mode::Project),
            [],
            "excluding a missing field is harmless"
        );
    }

    #[test]
    fn unknown_operators_with_a_guess() {
        let p = problems("{ age: { $gtt: 5 } }", Mode::Filter);
        assert_eq!(
            p,
            [(
                "$gtt".into(),
                Message::UnknownOperator {
                    op: "$gtt".into(),
                    suggestion: Some("$gt".into())
                }
            )]
        );
        assert!(
            matches!(&problems("{ $orr: [] }", Mode::Filter)[0].1, Message::UnknownOperator { suggestion: Some(s), .. } if s == "$or")
        );
        assert!(
            matches!(&problems("[{ $mach: {} }]", Mode::Pipeline)[0].1, Message::UnknownOperator { suggestion: Some(s), .. } if s == "$match")
        );
        assert!(matches!(
            &problems("{ $sett: { a: 1 } }", Mode::Update)[0].1,
            Message::UnknownOperator { .. }
        ));
    }

    #[test]
    fn values_of_the_wrong_type() {
        let p = problems("{ age: '30' }", Mode::Filter);
        assert_eq!(
            p,
            [(
                "'30'".into(),
                Message::TypeMismatch {
                    field: "age".into(),
                    types: "int".into(),
                    value: "string".into()
                }
            )]
        );
        assert_eq!(problems("{ age: { $gt: '5' } }", Mode::Filter).len(), 1);
        assert_eq!(problems("{ age: { $in: [1, 'x'] } }", Mode::Filter)[0].0, "'x'");
        assert_eq!(problems("{ createdAt: { $gt: 5 } }", Mode::Filter).len(), 1);
        assert_eq!(
            problems("{ age: NumberLong(5), name: null, tags: 'x' }", Mode::Filter),
            [],
            "families, null, array elements"
        );
    }

    #[test]
    fn operator_arguments_follow_the_specification() {
        let p = problems("{ age: { $in: 5 } }", Mode::Filter);
        assert!(
            matches!(&p[0].1, Message::ArgumentType { op, .. } if op == "$in"),
            "{p:?}"
        );
        assert!(matches!(
            &problems("{ tags: { $size: 'x' } }", Mode::Filter)[0].1,
            Message::ArgumentType { .. }
        ));
        assert!(matches!(
            &problems("{ age: { $mod: [4] } }", Mode::Filter)[0].1,
            Message::ArgumentCount { count: 2, .. }
        ));
        assert_eq!(
            problems(
                "{ age: { $exists: 1 }, name: { $type: ['string', 'null'], $regex: 'a' } }",
                Mode::Filter
            ),
            []
        );
    }

    #[test]
    fn pipelines_check_each_stage_against_the_fields_before_it() {
        let text = "[{ $group: { _id: '$address.city', n: { $sum: 1 } } }, { $match: { age: 1 } }]";
        assert_eq!(
            problems(text, Mode::Pipeline)[0].1,
            unknown_field("age", None),
            "gone after $group"
        );
        assert_eq!(
            problems("[{ $project: { up: { $toUpper: '$nmae' } } }]", Mode::Pipeline)[0].0,
            "$nmae"
        );
        assert!(matches!(
            &problems("[{ $match: {}, $limit: 1 }]", Mode::Pipeline)[0].1,
            Message::OneOperatorPerStage
        ));
        let p = problems("[{ $unwind: { pth: '$tags' } }]", Mode::Pipeline);
        assert!(
            p.iter()
                .any(|(_, m)| matches!(m, Message::UnknownArgument { suggestion: Some(s), .. } if s == "path")),
            "{p:?}"
        );
        assert!(
            p.iter()
                .any(|(_, m)| matches!(m, Message::MissingArgument { arg, .. } if arg == "path")),
            "{p:?}"
        );
        assert_eq!(
            problems("[{ $unwind: '$tags' }, { $sort: { tags: 1 } }]", Mode::Pipeline),
            []
        );
    }

    #[test]
    fn half_typed_text_checks_what_parses() {
        assert_eq!(problems("{ emial: 1, age: { $g", Mode::Filter)[0].0, "emial");
        assert_eq!(problems("", Mode::Filter), []);
        assert_eq!(problems("{ age: ", Mode::Filter), []);
    }

    #[test]
    fn nothing_without_a_sample() {
        assert_eq!(check("{ anything: 1 }", Mode::Filter, &Schema::default()), []);
        assert!(
            !check("{ a: { $gtt: 1 } }", Mode::Filter, &Schema::default()).is_empty(),
            "operators still checked"
        );
    }

    #[test]
    fn messages_read_well() {
        let m = unknown_field("emial", Some("email"));
        assert_eq!(m.text(), "emial isn't in the sampled documents. Did you mean email?");
    }

    #[test]
    fn a_complete_schema_says_the_field_is_nowhere() {
        let mut complete = schema();
        complete.complete = true;
        let p = check("{ emial: 1 }", Mode::Filter, &complete);
        assert!(matches!(&p[0].message, Message::UnknownField { everywhere: true, .. }));
        assert_eq!(
            p[0].message.text(),
            "emial isn't in any document of the collection. Did you mean email?"
        );
        assert!(matches!(
            &check("{ emial: 1 }", Mode::Filter, &schema())[0].message,
            Message::UnknownField { everywhere: false, .. }
        ));
    }
}
