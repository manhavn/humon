//! Canonical extended JSON to mongo shell text, as shown and edited in the UI.

use serde_json::{Map, Value};

fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

fn is_ident(k: &str) -> bool {
    let mut chars = k.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// `ISODate("…")`, or `ISODate(<millis>)` for dates ISO 8601 can't express
/// (past year 9999); the query parser reads both back to the same instant.
fn date_shell(ms: &str) -> String {
    match ms
        .parse::<i64>()
        .ok()
        .map(|ms| mongodb::bson::DateTime::from_millis(ms).try_to_rfc3339_string())
    {
        Some(Ok(iso)) => format!("ISODate({})", quote(&iso)),
        _ => format!("ISODate({ms})"),
    }
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
            date_shell(ms)
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

/// Whether an object is an extended JSON wrapper (`{"$oid": …}` and the
/// like) that stands for one typed value rather than a document.
pub fn is_wrapper(obj: &Map<String, Value>) -> bool {
    typed(obj).is_some()
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
            let lines: Vec<String> = items
                .iter()
                .map(|x| format!("{inner}{}", to_shell(x, &inner)))
                .collect();
            format!("[\n{}\n{indent}]", lines.join(",\n"))
        }
        Value::String(s) => quote(s),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{is_wrapper, to_shell};
    use serde_json::json;

    #[test]
    fn layout_is_two_space_indented_with_inline_empties() {
        let doc = json!({ "a": 1, "nested": { "b": [1, 2], "e": {}, "l": [] } });
        assert_eq!(
            to_shell(&doc, ""),
            "{\n  a: 1,\n  nested: {\n    b: [\n      1,\n      2\n    ],\n    e: {},\n    l: []\n  }\n}"
        );
    }

    #[test]
    fn keys_are_quoted_only_when_needed() {
        let doc = json!({ "plain": 1, "_id": 2, "$set": 3, "a.b": 4, "with space": 5, "1st": 6, "çay": 7 });
        assert_eq!(
            to_shell(&doc, ""),
            "{\n  plain: 1,\n  _id: 2,\n  $set: 3,\n  \"a.b\": 4,\n  \"with space\": 5,\n  \"1st\": 6,\n  çay: 7\n}"
        );
    }

    #[test]
    fn typed_values_use_shell_helpers() {
        let cases = [
            (
                json!({ "$oid": "65a1b2c3d4e5f60718293a4b" }),
                r#"ObjectId("65a1b2c3d4e5f60718293a4b")"#,
            ),
            (
                json!({ "$date": { "$numberLong": "0" } }),
                r#"ISODate("1970-01-01T00:00:00Z")"#,
            ),
            (
                json!({ "$date": { "$numberLong": "253402300800000" } }),
                "ISODate(253402300800000)",
            ),
            (json!({ "$numberInt": "7" }), "7"),
            (
                json!({ "$numberLong": "9007199254740993" }),
                r#"NumberLong("9007199254740993")"#,
            ),
            (json!({ "$numberDouble": "1.0" }), "1.0"),
            (json!({ "$numberDouble": "0.1" }), "0.1"),
            (json!({ "$numberDouble": "-Infinity" }), "-Infinity"),
            (json!({ "$numberDecimal": "1.10" }), r#"NumberDecimal("1.10")"#),
            (
                json!({ "$binary": { "base64": "EhISEhISEhISEhISEhISEg==", "subType": "04" } }),
                r#"UUID("12121212-1212-1212-1212-121212121212")"#,
            ),
            (
                json!({ "$binary": { "base64": "AQID", "subType": "00" } }),
                r#"BinData(0, "AQID")"#,
            ),
            (
                json!({ "$regularExpression": { "pattern": "^a", "options": "i" } }),
                "/^a/i",
            ),
            (json!({ "$timestamp": { "t": 10, "i": 2 } }), "Timestamp(10, 2)"),
            (json!({ "$minKey": 1 }), "MinKey()"),
        ];
        for (value, shell) in cases {
            assert_eq!(to_shell(&value, ""), shell, "for {value}");
        }
    }

    #[test]
    fn a_uuid_subtype_with_the_wrong_length_stays_binary() {
        // 15 bytes can't be a UUID; showing it as one would not parse back.
        let value = json!({ "$binary": { "base64": "EhISEhISEhISEhISEhIS", "subType": "04" } });
        assert_eq!(to_shell(&value, ""), r#"BinData(4, "EhISEhISEhISEhISEhIS")"#);
    }

    #[test]
    fn wrappers_are_exactly_the_typed_values() {
        let obj = |v: serde_json::Value| v.as_object().unwrap().clone();
        assert!(is_wrapper(&obj(json!({ "$oid": "65a1b2c3d4e5f60718293a4b" }))));
        assert!(is_wrapper(&obj(json!({ "$date": { "$numberLong": "0" } }))));
        assert!(is_wrapper(&obj(json!({ "$numberLong": "5" }))));
        // A field that merely starts with `$` is a normal (expandable) object.
        assert!(!is_wrapper(&obj(json!({ "$foo": 1, "x": 2 }))));
        assert!(!is_wrapper(&obj(json!({ "$set": { "a": 1 } }))));
        assert!(!is_wrapper(&obj(json!({}))));
        assert!(!is_wrapper(&obj(json!({ "a": 1 }))));
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(to_shell(&json!("a \"q\" \\ \n"), ""), r#""a \"q\" \\ \n""#);
    }
}
