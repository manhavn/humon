//! Completion in the shell: collection names after `db.`, methods after
//! `db.users.`, and inside a call's object argument (`find({ … })`,
//! `aggregate([ … ])`) the same suggestions as the query boxes, with that
//! collection's schema.

use super::{Completions, Kind, Mode, Suggestion, complete};
use crate::schema::Schema;

/// Database methods in the shell's prelude.
pub const DB_METHODS: &[&str] = &[
    "adminCommand",
    "createCollection",
    "dropDatabase",
    "getCollection",
    "getCollectionInfos",
    "getCollectionNames",
    "getName",
    "getSiblingDB",
    "runCommand",
    "serverStatus",
    "stats",
    "version",
];

/// Collection methods in the shell's prelude, most used first.
pub const COLLECTION_METHODS: &[&str] = &[
    "find",
    "findOne",
    "aggregate",
    "countDocuments",
    "insertOne",
    "insertMany",
    "updateOne",
    "updateMany",
    "deleteOne",
    "deleteMany",
    "replaceOne",
    "distinct",
    "estimatedDocumentCount",
    "findOneAndUpdate",
    "findOneAndReplace",
    "findOneAndDelete",
    "createIndex",
    "getIndexes",
    "dropIndex",
    "drop",
    "renameCollection",
    "getName",
    "getFullName",
];

/// Cursor methods in the shell's prelude: modifiers first, then readers.
pub const CURSOR_METHODS: &[&str] = &[
    "sort",
    "limit",
    "skip",
    "projection",
    "project",
    "hint",
    "batchSize",
    "maxTimeMS",
    "toArray",
    "forEach",
    "map",
    "pretty",
    "hasNext",
    "next",
    "itcount",
    "close",
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tok {
    Ident,
    Str,
    Punct(char),
}

#[derive(Clone, Debug)]
struct Token {
    kind: Tok,
    range: std::ops::Range<usize>,
}

/// JavaScript tokens up to `end`: identifiers, strings and punctuation;
/// comments, numbers and regex literals are skipped.
fn tokens(src: &str) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let bytes = src.as_bytes();
    let mut chars = src.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            '/' if bytes.get(i + 1) == Some(&b'/') => {
                while chars.peek().is_some_and(|&(_, d)| d != '\n') {
                    chars.next();
                }
            }
            '/' if bytes.get(i + 1) == Some(&b'*') => {
                chars.next();
                let mut star = false;
                for (_, d) in chars.by_ref() {
                    if star && d == '/' {
                        break;
                    }
                    star = d == '*';
                }
            }
            '/' if out
                .last()
                .is_none_or(|t| matches!(t.kind, Tok::Punct(p) if "(,=:[!&|?{};".contains(p))) =>
            {
                let (mut escaped, mut class) = (false, false);
                for (_, d) in chars.by_ref() {
                    match d {
                        _ if escaped => escaped = false,
                        '\\' => escaped = true,
                        '[' => class = true,
                        ']' => class = false,
                        '/' if !class => break,
                        _ => {}
                    }
                }
                while chars.peek().is_some_and(|&(_, d)| d.is_ascii_alphabetic()) {
                    chars.next();
                }
            }
            '"' | '\'' | '`' => {
                let mut escaped = false;
                let mut end = src.len();
                for (j, d) in chars.by_ref() {
                    if escaped {
                        escaped = false;
                    } else if d == '\\' {
                        escaped = true;
                    } else if d == c {
                        end = j + 1;
                        break;
                    }
                }
                out.push(Token {
                    kind: Tok::Str,
                    range: i..end,
                });
            }
            c if c.is_alphabetic() || c == '_' || c == '$' => {
                let mut end = src.len();
                while let Some(&(j, d)) = chars.peek() {
                    if !(d.is_alphanumeric() || d == '_' || d == '$') {
                        end = j;
                        break;
                    }
                    chars.next();
                }
                out.push(Token {
                    kind: Tok::Ident,
                    range: i..end,
                });
            }
            c if c.is_ascii_digit() => {
                while chars.peek().is_some_and(|&(_, d)| d.is_alphanumeric() || d == '.') {
                    chars.next();
                }
            }
            c => out.push(Token {
                kind: Tok::Punct(c),
                range: i..i + c.len_utf8(),
            }),
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
enum Link {
    Prop(String),
    /// A call, with its first argument when that is a string literal.
    Call(String, Option<String>),
}

/// The member chain ending at token `end` (an identifier): `db.users.find(…).sort`
/// is `db`, `users`, `find(…)`, `sort`.
fn chain(src: &str, toks: &[Token], end: usize) -> Vec<Link> {
    let text = |t: &Token| src[t.range.clone()].to_string();
    let unquote = |t: &Token| {
        let s = &src[t.range.clone()];
        s.get(1..s.len().saturating_sub(1)).unwrap_or_default().to_string()
    };
    let mut links = vec![Link::Prop(text(&toks[end]))];
    let mut i = end;
    while i >= 2 && toks[i - 1].kind == Tok::Punct('.') {
        let prev = &toks[i - 2];
        match prev.kind {
            Tok::Ident => {
                links.push(Link::Prop(text(prev)));
                i -= 2;
            }
            Tok::Punct(')') => {
                // Back to the matching `(` and the name before it.
                let mut depth = 0;
                let mut k = i - 2;
                loop {
                    match toks[k].kind {
                        Tok::Punct(')') => depth += 1,
                        Tok::Punct('(') => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    if k == 0 {
                        return Vec::new();
                    }
                    k -= 1;
                }
                if k == 0 || toks[k - 1].kind != Tok::Ident {
                    break;
                }
                let arg = toks.get(k + 1).filter(|t| t.kind == Tok::Str).map(unquote);
                links.push(Link::Call(text(&toks[k - 1]), arg));
                i = k - 1;
            }
            _ => break,
        }
    }
    links.reverse();
    links
}

/// Where a chain points: its database (if not the current one), collection
/// and the calls after it.
struct Target {
    db: Option<String>,
    coll: Option<String>,
    rest: Vec<Link>,
}

fn target(links: &[Link]) -> Option<Target> {
    let mut it = links.iter().peekable();
    if it.next() != Some(&Link::Prop("db".into())) {
        return None;
    }
    let mut db = None;
    if let Some(Link::Call(name, Some(arg))) = it.peek()
        && name == "getSiblingDB"
    {
        db = Some(arg.clone());
        it.next();
    }
    let coll = match it.peek() {
        Some(Link::Prop(name)) if !DB_METHODS.contains(&name.as_str()) => Some(name.clone()),
        Some(Link::Call(name, Some(arg))) if name == "getCollection" => Some(arg.clone()),
        _ => None,
    };
    if coll.is_some() {
        it.next();
    }
    Some(Target {
        db,
        coll,
        rest: it.cloned().collect(),
    })
}

/// What the call's argument at `index` holds.
fn arg_mode(method: &str, index: usize) -> Option<Mode> {
    Some(match (method, index) {
        ("find" | "findOne", 0) => Mode::Filter,
        ("find" | "findOne", 1) => Mode::Project,
        (
            "countDocuments" | "deleteOne" | "deleteMany" | "findOneAndDelete" | "findOneAndReplace" | "replaceOne"
            | "updateOne" | "updateMany" | "findOneAndUpdate",
            0,
        ) => Mode::Filter,
        ("distinct", 1) => Mode::Filter,
        ("updateOne" | "updateMany" | "findOneAndUpdate", 1) => Mode::Update,
        ("aggregate" | "watch", 0) => Mode::Pipeline,
        ("sort" | "createIndex" | "hint", 0) => Mode::Sort,
        ("projection" | "project", 0) => Mode::Project,
        _ => return None,
    })
}

/// What the cursor is at, and what the suggestions need.
#[derive(Debug, PartialEq)]
pub enum Spot {
    /// `use |`
    Databases,
    /// `show |`
    Show,
    /// `db.|`: the collections of this database (None: the current one).
    Collections(Option<String>),
    CollectionMethods,
    CursorMethods,
    /// Inside the object argument starting at `start`, read in `mode`.
    Arg {
        db: Option<String>,
        coll: String,
        mode: Mode,
        start: usize,
    },
    Nothing,
}

fn word_start(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| c.is_alphanumeric() || c == '_' || c == '$')
        .last()
        .map_or(cursor, |(i, _)| i)
}

pub fn spot(text: &str, cursor: usize) -> Spot {
    let cursor = cursor.min(text.len());
    let start = word_start(text, cursor);
    let line = text[..start].rsplit('\n').next().unwrap_or_default().trim();
    match line {
        "use" => return Spot::Databases,
        "show" => return Spot::Show,
        _ => {}
    }
    let toks = tokens(&text[..start]);
    // `db.` / `db.users.` / `db.users.find().`: a member being typed.
    if toks.last().is_some_and(|t| t.kind == Tok::Punct('.')) && toks.len() >= 2 {
        let before = toks.len() - 2;
        let links = match toks[before].kind {
            Tok::Ident => chain(text, &toks[..before + 1], before),
            Tok::Punct(')') => {
                // A call ends the chain; read it by pretending a name follows.
                let mut with_name = toks.clone();
                with_name.push(Token {
                    kind: Tok::Ident,
                    range: start..start,
                });
                let mut links = chain(text, &with_name, with_name.len() - 1);
                links.pop();
                links
            }
            _ => Vec::new(),
        };
        return match target(&links) {
            Some(t) if t.coll.is_none() && t.rest.is_empty() => Spot::Collections(t.db),
            Some(t) if t.coll.is_some() && t.rest.is_empty() => Spot::CollectionMethods,
            Some(t) if t.coll.is_some() && matches!(t.rest.last(), Some(Link::Call(..))) => Spot::CursorMethods,
            _ => Spot::Nothing,
        };
    }
    // Inside a call's argument: find the innermost open `(`.
    let mut stack: Vec<(usize, usize, usize)> = Vec::new(); // (token index, arg index, arg start)
    let mut brackets: Vec<char> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            Tok::Punct('(') => {
                stack.push((i, 0, t.range.end));
                brackets.push('(');
            }
            Tok::Punct('{' | '[') => brackets.push('{'),
            // The guard pops either way; only a closed paren ends a call.
            Tok::Punct(')') if brackets.pop() == Some('(') => {
                stack.pop();
            }
            Tok::Punct('}' | ']') => {
                brackets.pop();
            }
            Tok::Punct(',') if brackets.last() == Some(&'(') => {
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                    top.2 = t.range.end;
                }
            }
            _ => {}
        }
    }
    let Some(&(open, index, arg_start)) = stack.last() else {
        return Spot::Nothing;
    };
    // Only inside an object or array literal in that argument.
    if brackets.last() == Some(&'(') || open == 0 || toks[open - 1].kind != Tok::Ident {
        return Spot::Nothing;
    }
    let links = chain(text, &toks, open - 1);
    let Some(Target {
        db,
        coll: Some(coll),
        rest,
    }) = target(&links)
    else {
        return Spot::Nothing;
    };
    let Some(Link::Prop(method)) = rest.last() else {
        return Spot::Nothing;
    };
    match arg_mode(method, index) {
        Some(mode) => Spot::Arg {
            db,
            coll,
            mode,
            start: arg_start,
        },
        None => Spot::Nothing,
    }
}

/// An object or array argument of a call the shell knows, for checking.
#[derive(Debug, PartialEq)]
pub struct Argument {
    pub db: Option<String>,
    pub coll: String,
    pub mode: Mode,
    pub range: std::ops::Range<usize>,
}

/// Every argument in `text` that holds MQL: `find({ … })`, `aggregate([ … ])`, ...
pub fn arguments(text: &str) -> Vec<Argument> {
    let toks = tokens(text);
    let mut out = Vec::new();
    // Open parens: (token index, argument index).
    let mut parens: Vec<(usize, usize)> = Vec::new();
    let mut brackets: Vec<char> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            Tok::Punct('(') => {
                parens.push((i, 0));
                brackets.push('(');
            }
            // The guard pops either way; only a closed paren ends a call.
            Tok::Punct(')') if brackets.pop() == Some('(') => {
                parens.pop();
            }
            Tok::Punct(',') if brackets.last() == Some(&'(') => {
                if let Some(top) = parens.last_mut() {
                    top.1 += 1;
                }
            }
            Tok::Punct(c @ ('{' | '[')) => {
                let starts_arg =
                    brackets.last() == Some(&'(') && i > 0 && matches!(toks[i - 1].kind, Tok::Punct('(' | ','));
                if starts_arg && let Some(&(open, index)) = parens.last() {
                    let resolved = (open > 0 && toks[open - 1].kind == Tok::Ident)
                        .then(|| target(&chain(text, &toks, open - 1)))
                        .flatten();
                    if let Some(Target {
                        db,
                        coll: Some(coll),
                        rest,
                    }) = resolved
                        && let Some(Link::Prop(method)) = rest.last()
                        && let Some(mode) = arg_mode(method, index)
                    {
                        // To the matching bracket, or the end of the text.
                        let mut depth = 0;
                        let mut end = text.len();
                        for u in &toks[i..] {
                            match u.kind {
                                Tok::Punct('{' | '[') => depth += 1,
                                Tok::Punct('}' | ']') => {
                                    depth -= 1;
                                    if depth == 0 {
                                        end = u.range.end;
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        out.push(Argument {
                            db,
                            coll,
                            mode,
                            range: t.range.start..end,
                        });
                    }
                }
                brackets.push(c);
            }
            Tok::Punct('}' | ']') => {
                brackets.pop();
            }
            _ => {}
        }
    }
    out
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// Suggestions for `spot`, given the data it needs.
pub fn complete_shell(
    text: &str,
    cursor: usize,
    spot: &Spot,
    names: &[String],
    schema: Option<&Schema>,
) -> Completions {
    let cursor = cursor.min(text.len());
    let start = word_start(text, cursor);
    let typed = &text[start..cursor];
    let end = text[cursor..]
        .char_indices()
        .find(|&(_, c)| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .map_or(text.len(), |(i, _)| cursor + i);
    let mut items: Vec<(u8, Suggestion)> = Vec::new();
    let mut push = |label: &str, insert: String, detail: &str, kind: Kind| {
        if let Some(r) = super::rank(label, typed) {
            items.push((
                r,
                Suggestion {
                    label: label.to_string(),
                    insert,
                    detail: detail.to_string(),
                    detail_key: None,
                    link: None,
                    kind,
                },
            ));
        }
    };
    match spot {
        Spot::Databases => names.iter().for_each(|n| push(n, n.clone(), "", Kind::Value)),
        Spot::Show => {
            for word in ["dbs", "collections"] {
                push(word, word.to_string(), "", Kind::Value);
            }
        }
        Spot::Collections(_) => {
            for name in names {
                let insert = if is_identifier(name) && !DB_METHODS.contains(&name.as_str()) {
                    name.clone()
                } else {
                    format!("getCollection({})", serde_json::to_string(name).unwrap_or_default())
                };
                push(name, insert, "collection", Kind::Field);
            }
            DB_METHODS
                .iter()
                .for_each(|m| push(m, m.to_string(), "db", Kind::Operator));
        }
        Spot::CollectionMethods => {
            COLLECTION_METHODS
                .iter()
                .for_each(|m| push(m, m.to_string(), "collection", Kind::Operator));
        }
        Spot::CursorMethods => {
            CURSOR_METHODS
                .iter()
                .for_each(|m| push(m, m.to_string(), "cursor", Kind::Operator));
        }
        Spot::Arg { mode, start: arg, .. } => {
            let empty = Schema::default();
            let mut found = complete(&text[*arg..], cursor - arg, *mode, schema.unwrap_or(&empty));
            found.replace = found.replace.start + arg..found.replace.end + arg;
            return found;
        }
        Spot::Nothing => {}
    }
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

    fn at(text: &str) -> (String, usize) {
        let cursor = text.find('|').expect("cursor marker");
        (text.replace('|', ""), cursor)
    }

    fn spot_at(text: &str) -> Spot {
        let (text, cursor) = at(text);
        spot(&text, cursor)
    }

    fn arg(coll: &str, mode: Mode, start: usize) -> Spot {
        Spot::Arg {
            db: None,
            coll: coll.into(),
            mode,
            start,
        }
    }

    #[test]
    fn use_and_show() {
        assert_eq!(spot_at("use |"), Spot::Databases);
        assert_eq!(spot_at("use sh|"), Spot::Databases);
        assert_eq!(spot_at("db.x.find()\nshow col|"), Spot::Show);
    }

    #[test]
    fn members_after_db_and_a_collection() {
        assert_eq!(spot_at("db.|"), Spot::Collections(None));
        assert_eq!(spot_at("db.us|"), Spot::Collections(None));
        assert_eq!(
            spot_at("db.getSiblingDB('logs').|"),
            Spot::Collections(Some("logs".into()))
        );
        assert_eq!(spot_at("db.users.|"), Spot::CollectionMethods);
        assert_eq!(spot_at("db.getCollection(\"a-b\").fi|"), Spot::CollectionMethods);
        assert_eq!(spot_at("db.users.find({ a: 1 }).|"), Spot::CursorMethods);
        assert_eq!(spot_at("db.users.find().sort({ a: 1 }).li|"), Spot::CursorMethods);
        assert_eq!(spot_at("x.|"), Spot::Nothing);
        assert_eq!(spot_at("db.getName().|"), Spot::Nothing);
    }

    #[test]
    fn object_arguments_of_calls() {
        assert_eq!(spot_at("db.users.find({ |"), arg("users", Mode::Filter, 14));
        assert_eq!(spot_at("db.users.find({ a: 1 }, { |"), arg("users", Mode::Project, 23));
        assert_eq!(
            spot_at("db.users.aggregate([{ $match: { |"),
            arg("users", Mode::Pipeline, 19)
        );
        assert_eq!(
            spot_at("db.users.updateOne({}, { $set: { |"),
            arg("users", Mode::Update, 22)
        );
        assert_eq!(spot_at("db.users.find().sort({ |"), arg("users", Mode::Sort, 21));
        assert_eq!(
            spot_at("db.getSiblingDB(\"logs\").events.find({ |"),
            Spot::Arg {
                db: Some("logs".into()),
                coll: "events".into(),
                mode: Mode::Filter,
                start: 36
            }
        );
        assert_eq!(spot_at("db.users.find(|"), Spot::Nothing, "no object yet");
        assert_eq!(
            spot_at("db.users.insertOne({ |"),
            Spot::Nothing,
            "a new document can hold anything"
        );
        assert_eq!(spot_at("print({ |"), Spot::Nothing);
    }

    #[test]
    fn strings_comments_and_finished_calls_are_skipped() {
        assert_eq!(spot_at("// db.users.find({\ndb.|"), Spot::Collections(None));
        assert_eq!(spot_at("db.users.find({ a: \"(\" , |"), arg("users", Mode::Filter, 14));
        assert_eq!(spot_at("db.users.find({ a: /[(]/ , |"), arg("users", Mode::Filter, 14));
        assert_eq!(
            spot_at("db.users.find({}); db.orders.find({ |"),
            arg("orders", Mode::Filter, 34)
        );
    }

    fn names(c: &Completions) -> Vec<&str> {
        c.items.iter().map(|s| s.label.as_str()).collect()
    }

    #[test]
    fn suggests_names_methods_and_fields() {
        let colls = vec!["users".to_string(), "order-items".to_string()];
        let (text, cursor) = at("db.|");
        let c = complete_shell(&text, cursor, &spot(&text, cursor), &colls, None);
        assert_eq!(&names(&c)[..2], ["users", "order-items"]);
        assert_eq!(c.items[1].insert, "getCollection(\"order-items\")");
        assert!(names(&c).contains(&"getSiblingDB"));

        let (text, cursor) = at("db.users.fi|");
        let c = complete_shell(&text, cursor, &spot(&text, cursor), &[], None);
        assert_eq!(&names(&c)[..2], ["find", "findOne"]);
        assert_eq!(c.replace, 9..11);

        let schema = crate::schema::infer(&[doc! { "name": "a", "age": 1 }]);
        let (text, cursor) = at("db.users.find({ na| })");
        let c = complete_shell(&text, cursor, &spot(&text, cursor), &[], Some(&schema));
        assert_eq!(names(&c), ["name"]);
        assert_eq!(c.replace, 16..18, "offsets are in the whole text");
    }

    #[test]
    fn method_lists_match_the_prelude() {
        let js = include_str!("../console/prelude.js");
        let methods = |marker: &str| -> Vec<String> {
            let start = js.find(marker).unwrap();
            let end = js[start..].find("\n  });").unwrap() + start;
            let mut names: Vec<String> = js[start..end]
                .lines()
                .filter_map(|l| {
                    let l = l.strip_prefix("    ")?;
                    let name: String = l.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
                    (!name.is_empty() && l[name.len()..].starts_with(['(', ':'])).then_some(name)
                })
                .collect();
            names.sort();
            names
        };
        let sorted = |list: &[&str]| {
            let mut v: Vec<String> = list.iter().map(|s| s.to_string()).collect();
            v.sort();
            v
        };
        assert_eq!(methods("Object.assign(Database.prototype, {"), sorted(DB_METHODS));
        assert_eq!(
            methods("Object.assign(Collection.prototype, {"),
            sorted(COLLECTION_METHODS)
        );
        assert_eq!(methods("Object.assign(Cursor.prototype, {"), sorted(CURSOR_METHODS));
    }

    #[test]
    fn finds_every_mql_argument() {
        let text =
            "db.users.find({ a: 1 }, { b: 1 }).sort({ c: -1 })\nprint({ x: 1 })\ndb.orders.aggregate([{ $match: {} }]";
        let found: Vec<(String, Mode, &str)> = arguments(text)
            .into_iter()
            .map(|a| (a.coll, a.mode, &text[a.range]))
            .collect();
        assert_eq!(
            found,
            [
                ("users".into(), Mode::Filter, "{ a: 1 }"),
                ("users".into(), Mode::Project, "{ b: 1 }"),
                ("users".into(), Mode::Sort, "{ c: -1 }"),
                ("orders".into(), Mode::Pipeline, "[{ $match: {} }]"),
            ]
        );
        let open = arguments("db.users.find({ a: ");
        assert_eq!(open[0].range, 14..19, "an unclosed argument runs to the end");
    }
}
