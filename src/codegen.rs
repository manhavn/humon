//! A tab's query as driver code: the filter, sort, projection and limit in
//! Node.js, Python, Go, Java or Rust, ready to paste into an application.

use std::collections::BTreeSet;

use mongodb::bson::spec::BinarySubtype;
use mongodb::bson::{Bson, DateTime, Document};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Node,
    Python,
    Go,
    Java,
    Rust,
}

impl Lang {
    pub const ALL: [Lang; 5] = [Lang::Node, Lang::Python, Lang::Go, Lang::Java, Lang::Rust];

    pub fn label(self) -> &'static str {
        match self {
            Lang::Node => "Node.js",
            Lang::Python => "Python",
            Lang::Go => "Go",
            Lang::Java => "Java",
            Lang::Rust => "Rust",
        }
    }
}

/// A find, parsed: what the tab's query bar holds.
pub struct Find {
    pub db: String,
    pub coll: String,
    pub filter: Document,
    pub sort: Document,
    pub projection: Document,
    pub limit: Option<i64>,
}

/// Imports and helper functions the rendered values need, in order.
#[derive(Default)]
struct Needs(BTreeSet<&'static str>);

impl Needs {
    fn add(&mut self, what: &'static str) {
        self.0.insert(what);
    }
}

fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

fn is_identifier(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// A double that reads as one in `lang`: `2.0`, not `2`; infinities and NaN
/// spelled the way each language has them.
fn float(lang: Lang, f: f64) -> String {
    if !f.is_finite() {
        let n = f.is_sign_negative();
        return match (lang, f.is_nan()) {
            (Lang::Node, true) => "NaN".into(),
            (Lang::Node, false) => if n { "-Infinity" } else { "Infinity" }.into(),
            (Lang::Python, true) => "float(\"nan\")".into(),
            (Lang::Python, false) => format!("float(\"{}inf\")", if n { "-" } else { "" }),
            (Lang::Go, true) => "math.NaN()".into(),
            (Lang::Go, false) => format!("math.Inf({})", if n { -1 } else { 1 }),
            (Lang::Java, true) => "Double.NaN".into(),
            (Lang::Java, false) => if n {
                "Double.NEGATIVE_INFINITY"
            } else {
                "Double.POSITIVE_INFINITY"
            }
            .into(),
            (Lang::Rust, true) => "f64::NAN".into(),
            (Lang::Rust, false) => if n { "f64::NEG_INFINITY" } else { "f64::INFINITY" }.into(),
        };
    }
    let s = f.to_string();
    if lang == Lang::Node || s.contains(['.', 'e', 'E']) {
        s
    } else {
        format!("{s}.0")
    }
}

/// A regex pattern inside `/.../`: slashes escaped, unless already escaped.
fn slashes_escaped(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut backslashes = 0;
    for c in pattern.chars() {
        if c == '/' && backslashes % 2 == 0 {
            out.push('\\');
        }
        backslashes = if c == '\\' { backslashes + 1 } else { 0 };
        out.push(c);
    }
    out
}

fn rfc3339(d: &DateTime) -> String {
    d.try_to_rfc3339_string()
        .unwrap_or_else(|_| d.timestamp_millis().to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn uuid_text(bytes: &[u8]) -> String {
    let h = hex(bytes);
    if h.len() != 32 {
        return h;
    }
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Date parts from an RFC 3339 string: year, month, day, hour, minute, second, microsecond.
fn date_parts(d: &DateTime) -> Option<[u32; 7]> {
    let s = rfc3339(d);
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<u32>().ok();
    let micros = s
        .get(19..)
        .and_then(|rest| rest.strip_prefix('.'))
        .map(|frac| {
            let digits: String = frac.chars().take_while(|c| c.is_ascii_digit()).collect();
            format!("{digits:0<6}")[..6].parse().unwrap_or(0)
        })
        .unwrap_or(0);
    Some([
        num(0..4)?,
        num(5..7)?,
        num(8..10)?,
        num(11..13)?,
        num(14..16)?,
        num(17..19)?,
        micros,
    ])
}

fn value(lang: Lang, v: &Bson, needs: &mut Needs) -> String {
    match lang {
        Lang::Node => node_value(v, needs),
        Lang::Python => python_value(v, needs),
        Lang::Go => go_value(v, needs),
        Lang::Java => java_value(v, needs),
        Lang::Rust => rust_value(v),
    }
}

fn node_value(v: &Bson, needs: &mut Needs) -> String {
    match v {
        Bson::Document(d) if d.is_empty() => "{}".into(),
        Bson::Document(d) => {
            let fields: Vec<String> = d
                .iter()
                .map(|(k, v)| {
                    let key = if is_identifier(k) { k.clone() } else { quote(k) };
                    format!("{key}: {}", node_value(v, needs))
                })
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        Bson::Array(a) => format!(
            "[{}]",
            a.iter().map(|v| node_value(v, needs)).collect::<Vec<_>>().join(", ")
        ),
        Bson::String(s) => quote(s),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => {
            needs.add("Long");
            format!("Long.fromString(\"{n}\")")
        }
        Bson::Double(f) => float(Lang::Node, *f),
        Bson::Decimal128(d) => {
            needs.add("Decimal128");
            format!("Decimal128.fromString(\"{d}\")")
        }
        Bson::Boolean(b) => b.to_string(),
        Bson::Null | Bson::Undefined => "null".into(),
        Bson::ObjectId(id) => {
            needs.add("ObjectId");
            format!("new ObjectId(\"{id}\")")
        }
        Bson::DateTime(d) => {
            let mut s = rfc3339(d);
            if !s.contains('.') {
                s = s.replace('Z', ".000Z");
            }
            format!("new Date(\"{s}\")")
        }
        Bson::RegularExpression(r) => format!("/{}/{}", slashes_escaped(&r.pattern), r.options),
        Bson::Binary(b) if b.subtype == BinarySubtype::Uuid => {
            needs.add("UUID");
            format!("new UUID(\"{}\")", uuid_text(&b.bytes))
        }
        Bson::Binary(b) => {
            needs.add("Binary");
            format!(
                "Binary.createFromBase64(\"{}\", {})",
                base64(&b.bytes),
                u8::from(b.subtype)
            )
        }
        Bson::Timestamp(t) => {
            needs.add("Timestamp");
            format!("new Timestamp({{ t: {}, i: {} }})", t.time, t.increment)
        }
        Bson::MinKey => {
            needs.add("MinKey");
            "new MinKey()".into()
        }
        Bson::MaxKey => {
            needs.add("MaxKey");
            "new MaxKey()".into()
        }
        other => quote(&other.to_string()),
    }
}

fn python_value(v: &Bson, needs: &mut Needs) -> String {
    match v {
        Bson::Document(d) if d.is_empty() => "{}".into(),
        Bson::Document(d) => {
            let fields: Vec<String> = d
                .iter()
                .map(|(k, v)| format!("{}: {}", quote(k), python_value(v, needs)))
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
        Bson::Array(a) => format!(
            "[{}]",
            a.iter().map(|v| python_value(v, needs)).collect::<Vec<_>>().join(", ")
        ),
        Bson::String(s) => quote(s),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => {
            needs.add("from bson Int64");
            format!("Int64({n})")
        }
        Bson::Double(f) => float(Lang::Python, *f),
        Bson::Decimal128(d) => {
            needs.add("from bson Decimal128");
            format!("Decimal128(\"{d}\")")
        }
        Bson::Boolean(true) => "True".into(),
        Bson::Boolean(false) => "False".into(),
        Bson::Null | Bson::Undefined => "None".into(),
        Bson::ObjectId(id) => {
            needs.add("from bson ObjectId");
            format!("ObjectId(\"{id}\")")
        }
        Bson::DateTime(d) => {
            needs.add("import datetime");
            let Some([y, mo, day, h, mi, sec, us]) = date_parts(d) else {
                return quote(&rfc3339(d));
            };
            let mut parts = vec![y, mo, day, h, mi];
            if sec != 0 || us != 0 {
                parts.push(sec);
            }
            if us != 0 {
                parts.push(us);
            }
            let parts: Vec<String> = parts.iter().map(u32::to_string).collect();
            format!("datetime.datetime({}, tzinfo=datetime.timezone.utc)", parts.join(", "))
        }
        Bson::RegularExpression(r) => {
            needs.add("import re");
            let flags: Vec<&str> = r
                .options
                .chars()
                .filter_map(|c| match c {
                    'i' => Some("re.IGNORECASE"),
                    'm' => Some("re.MULTILINE"),
                    's' => Some("re.DOTALL"),
                    'x' => Some("re.VERBOSE"),
                    _ => None,
                })
                .collect();
            let pattern = if r.pattern.contains('"') || r.pattern.ends_with('\\') {
                quote(&r.pattern)
            } else {
                format!("r\"{}\"", r.pattern)
            };
            if flags.is_empty() {
                format!("re.compile({pattern})")
            } else {
                format!("re.compile({pattern}, {})", flags.join(" | "))
            }
        }
        Bson::Binary(b) if b.subtype == BinarySubtype::Uuid => {
            needs.add("from bson Binary");
            needs.add("import uuid");
            format!("Binary.from_uuid(uuid.UUID(\"{}\"))", uuid_text(&b.bytes))
        }
        Bson::Binary(b) => {
            needs.add("from bson Binary");
            needs.add("import base64");
            format!(
                "Binary(base64.b64decode(\"{}\"), {})",
                base64(&b.bytes),
                u8::from(b.subtype)
            )
        }
        Bson::Timestamp(t) => {
            needs.add("from bson Timestamp");
            format!("Timestamp({}, {})", t.time, t.increment)
        }
        Bson::MinKey => {
            needs.add("from bson MinKey");
            "MinKey()".into()
        }
        Bson::MaxKey => {
            needs.add("from bson MaxKey");
            "MaxKey()".into()
        }
        other => quote(&other.to_string()),
    }
}

fn go_value(v: &Bson, needs: &mut Needs) -> String {
    match v {
        Bson::Document(d) => {
            let fields: Vec<String> = d
                .iter()
                .map(|(k, v)| format!("{{{}, {}}}", quote(k), go_value(v, needs)))
                .collect();
            format!("bson.D{{{}}}", fields.join(", "))
        }
        Bson::Array(a) => format!(
            "bson.A{{{}}}",
            a.iter().map(|v| go_value(v, needs)).collect::<Vec<_>>().join(", ")
        ),
        Bson::String(s) => quote(s),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => format!("int64({n})"),
        Bson::Double(f) => float(Lang::Go, *f),
        Bson::Decimal128(d) => {
            needs.add("decimal");
            format!("decimal(\"{d}\")")
        }
        Bson::Boolean(b) => b.to_string(),
        Bson::Null | Bson::Undefined => "nil".into(),
        Bson::ObjectId(id) => {
            needs.add("objectID");
            format!("objectID(\"{id}\")")
        }
        Bson::DateTime(d) => {
            needs.add("mustTime");
            format!("mustTime(\"{}\")", rfc3339(d))
        }
        Bson::RegularExpression(r) => {
            format!(
                "bson.Regex{{Pattern: {}, Options: {}}}",
                quote(&r.pattern),
                quote(&r.options)
            )
        }
        Bson::Binary(b) if b.subtype == BinarySubtype::Uuid => {
            needs.add("uuidBinary");
            format!("uuidBinary(\"{}\")", uuid_text(&b.bytes))
        }
        Bson::Binary(b) => format!(
            "bson.Binary{{Subtype: {}, Data: []byte{{{}}}}}",
            u8::from(b.subtype),
            b.bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")
        ),
        Bson::Timestamp(t) => format!("bson.Timestamp{{T: {}, I: {}}}", t.time, t.increment),
        Bson::MinKey => "bson.MinKey{}".into(),
        Bson::MaxKey => "bson.MaxKey{}".into(),
        other => quote(&other.to_string()),
    }
}

fn java_doc(d: &Document, top: bool, needs: &mut Needs) -> String {
    let mut fields = d.iter();
    let Some((k, v)) = fields.next() else {
        return "new Document()".into();
    };
    let mut out = format!("new Document({}, {})", quote(k), java_value(v, needs));
    for (k, v) in fields {
        let sep = if top { "\n        " } else { "" };
        out.push_str(&format!("{sep}.append({}, {})", quote(k), java_value(v, needs)));
    }
    out
}

fn java_value(v: &Bson, needs: &mut Needs) -> String {
    match v {
        Bson::Document(d) => java_doc(d, false, needs),
        Bson::Array(a) => format!(
            "Arrays.asList({})",
            a.iter().map(|v| java_value(v, needs)).collect::<Vec<_>>().join(", ")
        ),
        Bson::String(s) => quote(s),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => format!("{n}L"),
        Bson::Double(f) => float(Lang::Java, *f),
        Bson::Decimal128(d) => format!("Decimal128.parse(\"{d}\")"),
        Bson::Boolean(b) => b.to_string(),
        Bson::Null | Bson::Undefined => "null".into(),
        Bson::ObjectId(id) => format!("new ObjectId(\"{id}\")"),
        Bson::DateTime(d) => format!("Date.from(Instant.parse(\"{}\"))", rfc3339(d)),
        Bson::RegularExpression(r) => {
            format!(
                "new BsonRegularExpression({}, {})",
                quote(&r.pattern),
                quote(&r.options)
            )
        }
        Bson::Binary(b) if b.subtype == BinarySubtype::Uuid => {
            format!("UUID.fromString(\"{}\")", uuid_text(&b.bytes))
        }
        Bson::Binary(b) => format!(
            "new Binary((byte) {}, Base64.getDecoder().decode(\"{}\"))",
            u8::from(b.subtype),
            base64(&b.bytes)
        ),
        Bson::Timestamp(t) => format!("new BsonTimestamp({}, {})", t.time, t.increment),
        Bson::MinKey => "new MinKey()".into(),
        Bson::MaxKey => "new MaxKey()".into(),
        other => quote(&other.to_string()),
    }
}

/// Inside `doc!`: nested documents are bare braces.
fn rust_value(v: &Bson) -> String {
    match v {
        Bson::Document(d) if d.is_empty() => "{}".into(),
        Bson::Document(d) => {
            let fields: Vec<String> = d
                .iter()
                .map(|(k, v)| format!("{}: {}", quote(k), rust_value(v)))
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        Bson::Array(a) => format!("[{}]", a.iter().map(rust_value).collect::<Vec<_>>().join(", ")),
        Bson::String(s) => quote(s),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => format!("{n}_i64"),
        Bson::Double(f) => float(Lang::Rust, *f),
        Bson::Decimal128(d) => format!("\"{d}\".parse::<Decimal128>()?"),
        Bson::Boolean(b) => b.to_string(),
        Bson::Null | Bson::Undefined => "Bson::Null".into(),
        Bson::ObjectId(id) => format!("ObjectId::parse_str(\"{id}\")?"),
        Bson::DateTime(d) => format!("DateTime::parse_rfc3339_str(\"{}\")?", rfc3339(d)),
        Bson::RegularExpression(r) => format!(
            "Regex {{ pattern: {}.into(), options: {}.into() }}",
            quote(&r.pattern),
            quote(&r.options)
        ),
        Bson::Binary(b) if b.subtype == BinarySubtype::Uuid => {
            format!("Uuid::parse_str(\"{}\")?", uuid_text(&b.bytes))
        }
        Bson::Binary(b) => format!(
            "Binary {{ subtype: BinarySubtype::from({}), bytes: vec![{}] }}",
            u8::from(b.subtype),
            b.bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")
        ),
        Bson::Timestamp(t) => format!("Timestamp {{ time: {}, increment: {} }}", t.time, t.increment),
        Bson::MinKey => "Bson::MinKey".into(),
        Bson::MaxKey => "Bson::MaxKey".into(),
        other => quote(&other.to_string()),
    }
}

fn doc(lang: Lang, d: &Document, needs: &mut Needs) -> String {
    match lang {
        Lang::Java => java_doc(d, true, needs),
        Lang::Rust if d.is_empty() => "doc! {}".into(),
        Lang::Rust => format!("doc! {}", rust_value(&Bson::Document(d.clone()))),
        _ => value(lang, &Bson::Document(d.clone()), needs),
    }
}

const GO_HELPERS: &[(&str, &str)] = &[
    (
        "objectID",
        "func objectID(hex string) bson.ObjectID { id, _ := bson.ObjectIDFromHex(hex); return id }",
    ),
    (
        "mustTime",
        "func mustTime(s string) time.Time { t, _ := time.Parse(time.RFC3339Nano, s); return t }",
    ),
    (
        "decimal",
        "func decimal(s string) bson.Decimal128 { d, _ := bson.ParseDecimal128(s); return d }",
    ),
    (
        "uuidBinary",
        "func uuidBinary(s string) bson.Binary { u, _ := uuid.Parse(s); return bson.Binary{Subtype: 4, Data: u[:]} }",
    ),
];

/// The find as code in `lang`.
pub fn to_code(lang: Lang, find: &Find) -> String {
    let mut needs = Needs::default();
    let filter = doc(lang, &find.filter, &mut needs);
    let sort = (!find.sort.is_empty()).then(|| doc(lang, &find.sort, &mut needs));
    let projection = (!find.projection.is_empty()).then(|| doc(lang, &find.projection, &mut needs));
    let (db, coll) = (quote(&find.db), quote(&find.coll));
    match lang {
        Lang::Node => {
            let mut code = format!("const cursor = client.db({db}).collection({coll})\n  .find({filter})");
            if let Some(s) = sort {
                code.push_str(&format!("\n  .sort({s})"));
            }
            if let Some(p) = projection {
                code.push_str(&format!("\n  .project({p})"));
            }
            if let Some(n) = find.limit {
                code.push_str(&format!("\n  .limit({n})"));
            }
            code.push(';');
            if needs.0.is_empty() {
                code
            } else {
                let names: Vec<&str> = needs.0.iter().copied().collect();
                format!("import {{ {} }} from \"mongodb\";\n\n{code}", names.join(", "))
            }
        }
        Lang::Python => {
            let mut code = match projection {
                Some(p) => format!("cursor = client[{db}][{coll}].find(\n    {filter},\n    {p},\n)"),
                None => format!("cursor = client[{db}][{coll}].find({filter})"),
            };
            if !find.sort.is_empty() {
                let pairs: Vec<String> = find
                    .sort
                    .iter()
                    .map(|(k, v)| format!("({}, {})", quote(k), python_value(v, &mut needs)))
                    .collect();
                code.push_str(&format!(".sort([{}])", pairs.join(", ")));
            }
            if let Some(n) = find.limit {
                code.push_str(&format!(".limit({n})"));
            }
            let from_bson: Vec<&str> = needs.0.iter().filter_map(|n| n.strip_prefix("from bson ")).collect();
            let mut imports: Vec<String> = Vec::new();
            if !from_bson.is_empty() {
                imports.push(format!("from bson import {}", from_bson.join(", ")));
            }
            imports.extend(
                needs
                    .0
                    .iter()
                    .filter(|n| n.starts_with("import "))
                    .map(|n| n.to_string()),
            );
            if imports.is_empty() {
                code
            } else {
                format!("{}\n\n{code}", imports.join("\n"))
            }
        }
        Lang::Go => {
            let mut code = format!("filter := {filter}\n");
            let mut opts: Vec<String> = Vec::new();
            if let Some(s) = sort {
                opts.push(format!("SetSort({s})"));
            }
            if let Some(p) = projection {
                opts.push(format!("SetProjection({p})"));
            }
            if let Some(n) = find.limit {
                opts.push(format!("SetLimit({n})"));
            }
            if opts.is_empty() {
                code.push_str(&format!(
                    "cursor, err := client.Database({db}).Collection({coll}).Find(ctx, filter)"
                ));
            } else {
                code.push_str(&format!("opts := options.Find().\n\t{}\n", opts.join(".\n\t")));
                code.push_str(&format!(
                    "cursor, err := client.Database({db}).Collection({coll}).Find(ctx, filter, opts)"
                ));
            }
            let helpers: Vec<&str> = GO_HELPERS
                .iter()
                .filter(|(name, _)| needs.0.contains(name))
                .map(|(_, f)| *f)
                .collect();
            if helpers.is_empty() {
                code
            } else {
                format!("{code}\n\n{}", helpers.join("\n"))
            }
        }
        Lang::Java => {
            let mut code = format!(
                "Bson filter = {filter};\nFindIterable<Document> cursor = client.getDatabase({db}).getCollection({coll})\n        .find(filter)"
            );
            if let Some(s) = sort {
                code.push_str(&format!("\n        .sort({s})"));
            }
            if let Some(p) = projection {
                code.push_str(&format!("\n        .projection({p})"));
            }
            if let Some(n) = find.limit {
                code.push_str(&format!("\n        .limit({n})"));
            }
            code.push(';');
            code
        }
        Lang::Rust => {
            let mut code =
                format!("let cursor = client.database({db}).collection::<Document>({coll})\n    .find({filter})");
            if let Some(s) = sort {
                code.push_str(&format!("\n    .sort({s})"));
            }
            if let Some(p) = projection {
                code.push_str(&format!("\n    .projection({p})"));
            }
            if let Some(n) = find.limit {
                code.push_str(&format!("\n    .limit({n})"));
            }
            code.push_str("\n    .await?;");
            code
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(filter: &str, sort: &str, projection: &str, limit: Option<i64>) -> Find {
        Find {
            db: "shop".into(),
            coll: "users".into(),
            filter: crate::query::parse_document(filter).unwrap(),
            sort: crate::query::parse_document(sort).unwrap(),
            projection: crate::query::parse_document(projection).unwrap(),
            limit,
        }
    }

    const TYPES: &str = r#"{ _id: ObjectId("65a1b2c3d4e5f60718293a4b"), at: { $gte: ISODate("2024-01-01T00:00:00Z") },
        big: NumberLong("9007199254740993"), price: NumberDecimal("10.50"), name: /^al/i, n: 1.5, ok: true, gone: null,
        tags: { $in: ["a", "b"] } }"#;

    #[test]
    fn node() {
        let code = to_code(
            Lang::Node,
            &find("{ age: { $gt: 30 } }", "{ age: -1 }", "{ name: 1 }", Some(20)),
        );
        assert_eq!(
            code,
            r#"const cursor = client.db("shop").collection("users")
  .find({ age: { $gt: 30 } })
  .sort({ age: -1 })
  .project({ name: 1 })
  .limit(20);"#
        );
        let code = to_code(Lang::Node, &find(TYPES, "", "", None));
        assert!(
            code.starts_with("import { Decimal128, Long, ObjectId } from \"mongodb\";\n\n"),
            "{code}"
        );
        for part in [
            r#"_id: new ObjectId("65a1b2c3d4e5f60718293a4b")"#,
            r#"at: { $gte: new Date("2024-01-01T00:00:00.000Z") }"#,
            r#"big: Long.fromString("9007199254740993")"#,
            r#"price: Decimal128.fromString("10.50")"#,
            "name: /^al/i",
            "n: 1.5",
            "gone: null",
            r#"tags: { $in: ["a", "b"] }"#,
        ] {
            assert!(code.contains(part), "{part} in\n{code}");
        }
    }

    #[test]
    fn python() {
        let code = to_code(
            Lang::Python,
            &find("{ age: { $gt: 30 } }", "{ age: -1, name: 1 }", "{ name: 1 }", Some(20)),
        );
        assert_eq!(
            code,
            r#"cursor = client["shop"]["users"].find(
    {"age": {"$gt": 30}},
    {"name": 1},
).sort([("age", -1), ("name", 1)]).limit(20)"#
        );
        let code = to_code(Lang::Python, &find(TYPES, "", "", None));
        for part in [
            "from bson import Decimal128, Int64, ObjectId",
            "import datetime",
            "import re",
            r#""_id": ObjectId("65a1b2c3d4e5f60718293a4b")"#,
            r#""at": {"$gte": datetime.datetime(2024, 1, 1, 0, 0, tzinfo=datetime.timezone.utc)}"#,
            r#""big": Int64(9007199254740993)"#,
            r#""price": Decimal128("10.50")"#,
            r#""name": re.compile(r"^al", re.IGNORECASE)"#,
            r#""ok": True"#,
            r#""gone": None"#,
        ] {
            assert!(code.contains(part), "{part} in\n{code}");
        }
    }

    #[test]
    fn go() {
        let code = to_code(
            Lang::Go,
            &find("{ age: { $gt: 30 } }", "{ age: -1 }", "{ name: 1 }", Some(20)),
        );
        assert_eq!(
            code,
            r#"filter := bson.D{{"age", bson.D{{"$gt", 30}}}}
opts := options.Find().
	SetSort(bson.D{{"age", -1}}).
	SetProjection(bson.D{{"name", 1}}).
	SetLimit(20)
cursor, err := client.Database("shop").Collection("users").Find(ctx, filter, opts)"#
        );
        let code = to_code(Lang::Go, &find(TYPES, "", "", None));
        for part in [
            r#"{"_id", objectID("65a1b2c3d4e5f60718293a4b")}"#,
            r#"{"at", bson.D{{"$gte", mustTime("2024-01-01T00:00:00Z")}}}"#,
            r#"{"big", int64(9007199254740993)}"#,
            r#"{"price", decimal("10.50")}"#,
            r#"{"name", bson.Regex{Pattern: "^al", Options: "i"}}"#,
            r#"{"tags", bson.D{{"$in", bson.A{"a", "b"}}}}"#,
            r#"{"gone", nil}"#,
            "func objectID(hex string) bson.ObjectID",
            "func mustTime(s string) time.Time",
            "func decimal(s string) bson.Decimal128",
        ] {
            assert!(code.contains(part), "{part} in\n{code}");
        }
        assert_eq!(
            to_code(Lang::Go, &find("", "", "", None)),
            r#"filter := bson.D{}
cursor, err := client.Database("shop").Collection("users").Find(ctx, filter)"#
        );
    }

    #[test]
    fn java() {
        let code = to_code(
            Lang::Java,
            &find("{ age: { $gt: 30 }, a: 1 }", "{ age: -1 }", "", Some(20)),
        );
        assert_eq!(
            code,
            r#"Bson filter = new Document("age", new Document("$gt", 30))
        .append("a", 1);
FindIterable<Document> cursor = client.getDatabase("shop").getCollection("users")
        .find(filter)
        .sort(new Document("age", -1))
        .limit(20);"#
        );
        let code = to_code(Lang::Java, &find(TYPES, "", "", None));
        for part in [
            r#"new Document("_id", new ObjectId("65a1b2c3d4e5f60718293a4b"))"#,
            r#"new Document("$gte", Date.from(Instant.parse("2024-01-01T00:00:00Z")))"#,
            r#".append("big", 9007199254740993L)"#,
            r#".append("price", Decimal128.parse("10.50"))"#,
            r#".append("name", new BsonRegularExpression("^al", "i"))"#,
            r#"new Document("$in", Arrays.asList("a", "b"))"#,
        ] {
            assert!(code.contains(part), "{part} in\n{code}");
        }
    }

    #[test]
    fn rust() {
        let code = to_code(
            Lang::Rust,
            &find("{ age: { $gt: 30 } }", "{ age: -1 }", "{ name: 1 }", Some(20)),
        );
        assert_eq!(
            code,
            r#"let cursor = client.database("shop").collection::<Document>("users")
    .find(doc! { "age": { "$gt": 30 } })
    .sort(doc! { "age": -1 })
    .projection(doc! { "name": 1 })
    .limit(20)
    .await?;"#
        );
        let code = to_code(Lang::Rust, &find(TYPES, "", "", None));
        for part in [
            r#""_id": ObjectId::parse_str("65a1b2c3d4e5f60718293a4b")?"#,
            r#""$gte": DateTime::parse_rfc3339_str("2024-01-01T00:00:00Z")?"#,
            r#""big": 9007199254740993_i64"#,
            r#""price": "10.50".parse::<Decimal128>()?"#,
            r#""name": Regex { pattern: "^al".into(), options: "i".into() }"#,
            r#""gone": Bson::Null"#,
        ] {
            assert!(code.contains(part), "{part} in\n{code}");
        }
    }

    #[test]
    fn odd_keys_and_strings_are_quoted_and_escaped() {
        let f = find(r#"{ "a.b": "say \"hi\"", "x-y": 1 }"#, "", "", None);
        assert!(to_code(Lang::Node, &f).contains(r#""a.b": "say \"hi\"", "x-y": 1"#));
        assert!(to_code(Lang::Python, &f).contains(r#""a.b": "say \"hi\"""#));
    }

    fn one(v: Bson) -> Find {
        let mut filter = Document::new();
        filter.insert("x", v);
        Find {
            db: "shop".into(),
            coll: "users".into(),
            filter,
            sort: Document::new(),
            projection: Document::new(),
            limit: None,
        }
    }

    #[test]
    fn infinity_and_nan_in_each_language() {
        let (inf, nan) = (Bson::Double(f64::INFINITY), Bson::Double(f64::NAN));
        assert!(to_code(Lang::Node, &one(inf.clone())).contains("x: Infinity"));
        assert!(to_code(Lang::Node, &one(nan.clone())).contains("x: NaN"));
        assert!(to_code(Lang::Python, &one(Bson::Double(f64::NEG_INFINITY))).contains(r#""x": float("-inf")"#));
        assert!(to_code(Lang::Go, &one(inf.clone())).contains(r#"{"x", math.Inf(1)}"#));
        assert!(to_code(Lang::Java, &one(nan)).contains("Double.NaN"));
        assert!(to_code(Lang::Rust, &one(inf)).contains(r#""x": f64::INFINITY"#));
    }

    #[test]
    fn node_regexes_escape_slashes_once() {
        let re = |p: &str| {
            one(Bson::RegularExpression(mongodb::bson::Regex {
                pattern: p.into(),
                options: "i".into(),
            }))
        };
        assert!(to_code(Lang::Node, &re("a/b")).contains(r"x: /a\/b/i"));
        assert!(
            to_code(Lang::Node, &re(r"a\/b")).contains(r"x: /a\/b/i"),
            "already escaped"
        );
        assert!(
            to_code(Lang::Node, &re(r"a\\/b")).contains(r"x: /a\\\/b/i"),
            "an escaped backslash, then a slash"
        );
    }

    #[test]
    fn base64_pads_and_uuids_get_dashes() {
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(uuid_text(&[0x12; 16]), "12121212-1212-1212-1212-121212121212");
        assert_eq!(uuid_text(&[0xab, 0xcd]), "abcd", "not a UUID: plain hex");
    }
}
