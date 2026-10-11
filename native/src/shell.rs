//! Port of src/lib/ejson.ts `toShell`: canonical extended JSON to mongo shell text.

use serde_json::{Map, Value};

fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

fn is_ident(k: &str) -> bool {
    let mut chars = k.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

fn date_text(ms: &str) -> String {
    ms.parse::<i64>()
        .ok()
        .and_then(|ms| mongodb::bson::DateTime::from_millis(ms).try_to_rfc3339_string().ok())
        .unwrap_or_else(|| ms.to_string())
}

fn uuid_text(b64: &str) -> Option<String> {
    let bin = mongodb::bson::Binary::from_base64(b64, None).ok()?;
    let h: String = bin.bytes.iter().map(|b| format!("{b:02x}")).collect();
    (h.len() == 32).then(|| format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]))
}

/// Shell form of a single-key extended JSON wrapper, if `obj` is one.
fn typed(obj: &Map<String, Value>) -> Option<String> {
    let (k, v) = obj.iter().next()?;
    let s = v.as_str().unwrap_or_default();
    Some(match k.as_str() {
        "$oid" => format!("ObjectId({})", quote(s)),
        "$date" => {
            let ms = v.get("$numberLong").and_then(Value::as_str).unwrap_or(s);
            format!("ISODate({})", quote(&date_text(ms)))
        }
        "$numberInt" => s.to_string(),
        "$numberLong" => format!("NumberLong({})", quote(s)),
        "$numberDouble" => match s.parse::<f64>() {
            Ok(f) if f.is_finite() && f.fract() == 0.0 => format!("{f:.1}"),
            Ok(f) if f.is_finite() => f.to_string(),
            _ => s.to_string(),
        },
        "$numberDecimal" => format!("NumberDecimal({})", quote(s)),
        "$binary" => {
            let b64 = v.get("base64").and_then(Value::as_str).unwrap_or_default();
            let sub = v.get("subType").and_then(Value::as_str).unwrap_or("00");
            match (sub, uuid_text(b64)) {
                ("04", Some(u)) => format!("UUID({})", quote(&u)),
                _ => format!("BinData({}, {})", u8::from_str_radix(sub, 16).unwrap_or(0), quote(b64)),
            }
        }
        "$regularExpression" => format!(
            "/{}/{}",
            v.get("pattern").and_then(Value::as_str).unwrap_or_default(),
            v.get("options").and_then(Value::as_str).unwrap_or_default()
        ),
        "$timestamp" => format!("Timestamp({}, {})", v["t"], v["i"]),
        "$minKey" => "MinKey()".into(),
        "$maxKey" => "MaxKey()".into(),
        _ => return None,
    })
}

pub fn to_shell(v: &Value, indent: &str) -> String {
    let inner = format!("{indent}  ");
    match v {
        Value::Object(obj) => {
            if let Some(t) = typed(obj) {
                return t;
            }
            if obj.is_empty() {
                return "{}".into();
            }
            let lines: Vec<String> = obj
                .iter()
                .map(|(k, val)| {
                    let key = if is_ident(k) { k.clone() } else { quote(k) };
                    format!("{inner}{key}: {}", to_shell(val, &inner))
                })
                .collect();
            format!("{{\n{}\n{indent}}}", lines.join(",\n"))
        }
        Value::Array(items) if items.is_empty() => "[]".into(),
        Value::Array(items) => {
            let lines: Vec<String> = items.iter().map(|x| format!("{inner}{}", to_shell(x, &inner))).collect();
            format!("[\n{}\n{indent}]", lines.join(",\n"))
        }
        Value::String(s) => quote(s),
        other => other.to_string(),
    }
}
