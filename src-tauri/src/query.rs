//! Parses the mongo shell's relaxed query syntax into BSON: unquoted keys,
//! single quotes, trailing commas, `/regex/flags`, and helpers such as
//! `ObjectId()`, `ISODate()` and `NumberLong()`. Canonical extended JSON
//! (`{"$oid": "..."}`) is accepted too, so values copied from the UI paste back in.

use mongodb::bson::oid::ObjectId;
use mongodb::bson::spec::BinarySubtype;
use mongodb::bson::{Binary, Bson, DateTime, Decimal128, Document, Regex, Timestamp, Uuid};

pub fn parse_document(src: &str) -> Result<Document, String> {
    if src.trim().is_empty() {
        return Ok(Document::new());
    }
    match parse_value(src)? {
        Bson::Document(d) => Ok(d),
        _ => Err("sorgu { ... } biçiminde bir nesne olmalı".into()),
    }
}

/// One document or an array of them, as typed into the insert editor.
pub fn parse_documents(src: &str) -> Result<Vec<Document>, String> {
    let not_doc = || "her öğe { ... } biçiminde bir doküman olmalı".to_string();
    match parse_value(src)? {
        Bson::Document(d) => Ok(vec![d]),
        Bson::Array(items) if !items.is_empty() => items
            .into_iter()
            .map(|v| match v {
                Bson::Document(d) => Ok(d),
                _ => Err(not_doc()),
            })
            .collect(),
        _ => Err(not_doc()),
    }
}

fn parse_value(src: &str) -> Result<Bson, String> {
    let mut p = Parser { src, pos: 0 };
    let value = p.value()?;
    p.ws();
    if p.pos < src.len() {
        return Err(p.err("fazladan karakter"));
    }
    Ok(value)
}

/// Single-key documents that extended JSON uses to spell typed values.
const EJSON_KEYS: &[&str] = &[
    "$oid",
    "$date",
    "$numberInt",
    "$numberLong",
    "$numberDouble",
    "$numberDecimal",
    "$uuid",
    "$binary",
    "$regularExpression",
    "$timestamp",
    "$minKey",
    "$maxKey",
];

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
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('/') if self.src[self.pos..].starts_with("//") => {
                    while !matches!(self.bump(), Some('\n') | None) {}
                }
                _ => return,
            }
        }
    }

    fn eat(&mut self, c: char) -> bool {
        self.ws();
        if self.peek() == Some(c) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(self.err(&format!("'{c}' bekleniyordu")))
        }
    }

    fn err(&self, msg: &str) -> String {
        let line = self.src[..self.pos].matches('\n').count() + 1;
        let col = self.src[..self.pos]
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .chars()
            .count()
            + 1;
        format!("{msg} (satır {line}, sütun {col})")
    }

    fn value(&mut self) -> Result<Bson, String> {
        self.ws();
        match self.peek() {
            None => Err(self.err("değer bekleniyordu")),
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('"' | '\'') => Ok(Bson::String(self.string()?)),
            Some('/') => self.regex(),
            Some(c) if c == '-' || c == '+' || c == '.' || c.is_ascii_digit() => self.number(),
            Some(c) if is_ident_start(c) => self.ident_value(),
            Some(c) => Err(self.err(&format!("beklenmeyen karakter '{c}'"))),
        }
    }

    fn object(&mut self) -> Result<Bson, String> {
        self.bump();
        let mut doc = Document::new();
        loop {
            if self.eat('}') {
                break;
            }
            let key = self.key()?;
            self.expect(':')?;
            let value = self.value()?;
            doc.insert(key, value);
            if !self.eat(',') {
                self.expect('}')?;
                break;
            }
        }
        if doc.len() == 1 && EJSON_KEYS.contains(&doc.keys().next().unwrap().as_str()) {
            // Round-trip through relaxed JSON so the bson crate decodes the typed value.
            return Bson::try_from(Bson::Document(doc).into_relaxed_extjson())
                .map_err(|e| self.err(&format!("geçersiz extended JSON: {e}")));
        }
        Ok(Bson::Document(doc))
    }

    fn array(&mut self) -> Result<Bson, String> {
        self.bump();
        let mut items = Vec::new();
        loop {
            if self.eat(']') {
                break;
            }
            items.push(self.value()?);
            if !self.eat(',') {
                self.expect(']')?;
                break;
            }
        }
        Ok(Bson::Array(items))
    }

    fn key(&mut self) -> Result<String, String> {
        self.ws();
        match self.peek() {
            Some('"' | '\'') => self.string(),
            // Dots are allowed unquoted so `a.b: 1` works without quoting the path.
            Some(c) if is_ident_start(c) || c.is_ascii_digit() => Ok(self.ident(true)),
            _ => Err(self.err("alan adı bekleniyordu")),
        }
    }

    fn ident(&mut self, allow_dots: bool) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if is_ident_start(c) || c.is_ascii_digit() || (allow_dots && c == '.') {
                self.bump();
            } else {
                break;
            }
        }
        self.src[start..self.pos].to_string()
    }

    fn string(&mut self) -> Result<String, String> {
        let quote = self.bump().unwrap();
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(self.err("kapanmamış metin")),
                Some(c) if c == quote => return Ok(out),
                Some('\\') => match self.bump() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('b') => out.push('\u{8}'),
                    Some('f') => out.push('\u{c}'),
                    Some('u') => out.push(self.unicode_escape()?),
                    Some(c) => out.push(c),
                    None => return Err(self.err("kapanmamış metin")),
                },
                Some(c) => out.push(c),
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let hi = self.hex4()?;
        if (0xD800..0xDC00).contains(&hi) && self.src[self.pos..].starts_with("\\u") {
            self.pos += 2;
            let lo = self.hex4()?;
            let code = 0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
            return char::from_u32(code).ok_or_else(|| self.err("geçersiz \\u kaçışı"));
        }
        char::from_u32(hi).ok_or_else(|| self.err("geçersiz \\u kaçışı"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .src
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.err("geçersiz \\u kaçışı"))?;
        let v = u32::from_str_radix(digits, 16).map_err(|_| self.err("geçersiz \\u kaçışı"))?;
        self.pos += 4;
        Ok(v)
    }

    fn regex(&mut self) -> Result<Bson, String> {
        self.bump();
        let start = self.pos;
        let mut in_class = false;
        loop {
            match self.bump() {
                None | Some('\n') => return Err(self.err("kapanmamış regex")),
                Some('\\') => {
                    self.bump();
                }
                Some('[') => in_class = true,
                Some(']') => in_class = false,
                Some('/') if !in_class => break,
                Some(_) => {}
            }
        }
        let pattern = self.src[start..self.pos - 1].to_string();
        let flags = self.ident(false);
        Ok(regex(pattern, &flags))
    }

    fn number(&mut self) -> Result<Bson, String> {
        let start = self.pos;
        let negative = self.peek() == Some('-');
        if matches!(self.peek(), Some('-' | '+')) {
            self.bump();
            if self.peek().is_some_and(is_ident_start) {
                let word = self.ident(false);
                return match word.as_str() {
                    "Infinity" if negative => Ok(Bson::Double(f64::NEG_INFINITY)),
                    "Infinity" => Ok(Bson::Double(f64::INFINITY)),
                    _ => Err(self.err("sayı bekleniyordu")),
                };
            }
        }
        while let Some(c) = self.peek() {
            let exponent_sign = matches!(c, '+' | '-')
                && matches!(self.src[..self.pos].chars().last(), Some('e' | 'E'));
            if c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E') || exponent_sign {
                self.bump();
            } else {
                break;
            }
        }
        let text = &self.src[start..self.pos];
        number_from_str(text).ok_or_else(|| self.err(&format!("geçersiz sayı '{text}'")))
    }

    fn ident_value(&mut self) -> Result<Bson, String> {
        let start = self.pos;
        let mut name = self.ident(false);
        match name.as_str() {
            "true" => return Ok(Bson::Boolean(true)),
            "false" => return Ok(Bson::Boolean(false)),
            "null" => return Ok(Bson::Null),
            "undefined" => return Ok(Bson::Undefined),
            "Infinity" => return Ok(Bson::Double(f64::INFINITY)),
            "NaN" => return Ok(Bson::Double(f64::NAN)),
            "new" => {
                self.ws();
                name = self.ident(false);
            }
            _ => {}
        }
        let args = if self.eat('(') {
            self.args()?
        } else {
            Vec::new()
        };
        self.call(&name, args).map_err(|e| {
            Parser {
                src: self.src,
                pos: start,
            }
            .err(&e)
        })
    }

    fn args(&mut self) -> Result<Vec<Bson>, String> {
        let mut args = Vec::new();
        loop {
            if self.eat(')') {
                return Ok(args);
            }
            args.push(self.value()?);
            if !self.eat(',') {
                self.expect(')')?;
                return Ok(args);
            }
        }
    }

    fn call(&self, name: &str, args: Vec<Bson>) -> Result<Bson, String> {
        let arg = |i: usize| args.get(i);
        Ok(match name {
            "ObjectId" | "ObjectID" => match arg(0) {
                None => Bson::ObjectId(ObjectId::new()),
                Some(Bson::String(s)) => Bson::ObjectId(
                    ObjectId::parse_str(s).map_err(|_| format!("geçersiz ObjectId: {s}"))?,
                ),
                _ => return Err("ObjectId metin almalı".into()),
            },
            "ISODate" | "Date" => match arg(0) {
                None => Bson::DateTime(DateTime::now()),
                Some(Bson::String(s)) => Bson::DateTime(parse_date(s)?),
                Some(n) => Bson::DateTime(DateTime::from_millis(as_i64(n)?)),
            },
            "NumberInt" | "Int32" => {
                let v = as_i64(arg(0).ok_or("NumberInt değer almalı")?)?;
                Bson::Int32(i32::try_from(v).map_err(|_| "NumberInt aralık dışında")?)
            }
            "NumberLong" | "Long" | "Int64" => {
                Bson::Int64(as_i64(arg(0).ok_or("NumberLong değer almalı")?)?)
            }
            "NumberDecimal" | "Decimal128" => {
                let text = match arg(0) {
                    Some(Bson::String(s)) => s.clone(),
                    Some(Bson::Int32(n)) => n.to_string(),
                    Some(Bson::Int64(n)) => n.to_string(),
                    Some(Bson::Double(n)) => n.to_string(),
                    _ => return Err("NumberDecimal değer almalı".into()),
                };
                Bson::Decimal128(
                    text.parse::<Decimal128>()
                        .map_err(|_| format!("geçersiz decimal: {text}"))?,
                )
            }
            "UUID" => match arg(0) {
                Some(Bson::String(s)) => {
                    Bson::from(Uuid::parse_str(s).map_err(|_| format!("geçersiz UUID: {s}"))?)
                }
                _ => return Err("UUID metin almalı".into()),
            },
            "BinData" => match (arg(0), arg(1)) {
                (Some(sub), Some(Bson::String(b64))) => {
                    let sub =
                        u8::try_from(as_i64(sub)?).map_err(|_| "geçersiz BinData alt tipi")?;
                    Bson::Binary(
                        Binary::from_base64(b64, BinarySubtype::from(sub))
                            .map_err(|e| e.to_string())?,
                    )
                }
                _ => return Err("BinData(altTip, \"base64\") biçiminde olmalı".into()),
            },
            "Timestamp" => match (arg(0), arg(1)) {
                (Some(t), Some(i)) => Bson::Timestamp(Timestamp {
                    time: u32::try_from(as_i64(t)?).map_err(|_| "geçersiz Timestamp")?,
                    increment: u32::try_from(as_i64(i)?).map_err(|_| "geçersiz Timestamp")?,
                }),
                _ => return Err("Timestamp(saniye, sayaç) biçiminde olmalı".into()),
            },
            "RegExp" => match (arg(0), arg(1)) {
                (Some(Bson::String(p)), None) => regex(p.clone(), ""),
                (Some(Bson::String(p)), Some(Bson::String(f))) => regex(p.clone(), f),
                _ => return Err("RegExp metin almalı".into()),
            },
            "MinKey" => Bson::MinKey,
            "MaxKey" => Bson::MaxKey,
            _ => return Err(format!("bilinmeyen ifade: {name}")),
        })
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn regex(pattern: String, flags: &str) -> Bson {
    // BSON requires regex options in alphabetical order.
    let mut options: Vec<char> = flags.chars().collect();
    options.sort_unstable();
    Bson::RegularExpression(Regex {
        pattern,
        options: options.into_iter().collect(),
    })
}

fn number_from_str(text: &str) -> Option<Bson> {
    let text = text.strip_prefix('+').unwrap_or(text);
    if !text.contains(['.', 'e', 'E'])
        && let Ok(i) = text.parse::<i64>()
    {
        return Some(match i32::try_from(i) {
            Ok(small) => Bson::Int32(small),
            Err(_) => Bson::Int64(i),
        });
    }
    text.parse::<f64>().ok().map(Bson::Double)
}

fn as_i64(v: &Bson) -> Result<i64, String> {
    match v {
        Bson::Int32(n) => Ok(*n as i64),
        Bson::Int64(n) => Ok(*n),
        Bson::Double(n) if n.fract() == 0.0 => Ok(*n as i64),
        Bson::String(s) => s
            .trim()
            .parse()
            .map_err(|_| format!("tam sayı bekleniyordu: {s}")),
        _ => Err("tam sayı bekleniyordu".into()),
    }
}

/// Accepts what people actually type into ISODate(): a bare date, a missing
/// timezone (treated as UTC), or a space between date and time.
fn parse_date(s: &str) -> Result<DateTime, String> {
    let mut text = s.trim().to_string();
    if text.len() == 10 {
        text.push_str("T00:00:00Z");
    }
    if text.as_bytes().get(10) == Some(&b' ') {
        text.replace_range(10..11, "T");
    }
    let time_part = text.get(11..).unwrap_or("");
    if !time_part.ends_with(['Z', 'z']) && !time_part.contains(['+', '-']) {
        text.push('Z');
    }
    DateTime::parse_rfc3339_str(&text).map_err(|_| format!("geçersiz tarih: {s}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    fn parse(s: &str) -> Document {
        parse_document(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn empty_input_is_empty_filter() {
        assert_eq!(parse("  "), doc! {});
    }

    #[test]
    fn shell_syntax() {
        let d = parse(
            "{ name: 'ali', age: { $gt: 18, }, 'a.b': -2.5, tags: [1, \"x\"], ok: true, n: null }",
        );
        assert_eq!(
            d,
            doc! { "name": "ali", "age": { "$gt": 18 }, "a.b": -2.5, "tags": [1, "x"], "ok": true, "n": null }
        );
    }

    #[test]
    fn unquoted_dotted_keys_and_key_order() {
        let d = parse("{ z: 1, address.city: -1, a: 1 }");
        assert_eq!(d.keys().collect::<Vec<_>>(), ["z", "address.city", "a"]);
    }

    #[test]
    fn helpers() {
        let d = parse(
            r#"{
                _id: ObjectId("65a1b2c3d4e5f60718293a4b"),
                at: ISODate("2024-01-02"),
                at2: new Date("2024-01-02T03:04:05.006+03:00"),
                big: NumberLong("9007199254740993"),
                small: NumberInt(7),
                dec: NumberDecimal("1.10"),
                id: UUID("123e4567-e89b-12d3-a456-426614174000"),
                re: /^ab\/c[/]d/mi,
                ts: Timestamp(10, 2),
            }"#,
        );
        assert_eq!(
            d.get_object_id("_id").unwrap().to_hex(),
            "65a1b2c3d4e5f60718293a4b"
        );
        assert_eq!(
            d.get_datetime("at").unwrap().timestamp_millis(),
            1704153600000
        );
        assert_eq!(
            d.get_datetime("at2").unwrap().timestamp_millis(),
            1704153845006
        );
        assert_eq!(d.get_i64("big").unwrap(), 9007199254740993);
        assert_eq!(d.get_i32("small").unwrap(), 7);
        assert_eq!(d.get("dec").unwrap().to_string(), "1.10");
        assert!(matches!(d.get("id"), Some(Bson::Binary(b)) if b.subtype == BinarySubtype::Uuid));
        let Some(Bson::RegularExpression(re)) = d.get("re") else {
            panic!()
        };
        assert_eq!(
            (re.pattern.as_str(), re.options.as_str()),
            (r"^ab\/c[/]d", "im")
        );
        assert_eq!(
            d.get_timestamp("ts").unwrap(),
            Timestamp {
                time: 10,
                increment: 2
            }
        );
    }

    #[test]
    fn extended_json_round_trips() {
        let d = parse(
            r#"{"_id": {"$oid": "65a1b2c3d4e5f60718293a4b"}, "n": {"$numberLong": "5"}, "at": {"$date": {"$numberLong": "0"}}}"#,
        );
        assert!(d.get_object_id("_id").is_ok());
        assert_eq!(d.get_i64("n").unwrap(), 5);
        assert_eq!(d.get_datetime("at").unwrap().timestamp_millis(), 0);
    }

    #[test]
    fn copied_document_parses_back() {
        // Exact output of the UI's "Kopyala" (ejson.ts toShell).
        let d = parse(
            r#"{
  _id: ObjectId("6acbe7d408caefbd7b5b22c6"),
  tags: [
    "a",
    "odd"
  ],
  address: {
    zip: 34005
  },
  createdAt: ISODate("2024-01-06T00:00:00.000Z"),
  balance: NumberDecimal("10.50"),
  big: NumberLong("9007199254740993"),
  uid: UUID("90dbbce7-3367-497d-8143-23a5b06357bf"),
  ratio: 1.0,
  re: /ab+c/i
}"#,
        );
        assert_eq!(d.len(), 9);
        assert_eq!(d.get("ratio"), Some(&Bson::Double(1.0)));
        assert_eq!(
            d.get_document("address").unwrap().get_i32("zip").unwrap(),
            34005
        );
    }

    #[test]
    fn insert_accepts_one_or_many() {
        assert_eq!(parse_documents("{ a: 1 }").unwrap().len(), 1);
        assert_eq!(parse_documents("[{ a: 1 }, { a: 2 },]").unwrap().len(), 2);
        assert!(parse_documents("[]").is_err());
        assert!(parse_documents("[{ a: 1 }, 2]").is_err());
    }

    #[test]
    fn errors_point_at_location() {
        let e = parse_document("{ a: 1,\n  b: }").unwrap_err();
        assert!(e.contains("satır 2"), "{e}");
        assert!(parse_document("{ a: Foo(1) }").unwrap_err().contains("Foo"));
        assert!(parse_document("[1]").is_err());
        assert!(parse_document("{ a: 1 } x").is_err());
    }
}
