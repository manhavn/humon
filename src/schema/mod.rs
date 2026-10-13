//! The shape of a collection, inferred from a sample of its documents: which
//! field paths exist, how often, and with which types. Feeds query completion
//! and validation.
//!
//! Documents are fed one at a time and dropped, so the sample size costs time
//! and server work, not memory; only the field list is kept.

use std::collections::HashMap;

use mongodb::bson::{Bson, Document};

mod stages;

/// How many documents to sample when the user hasn't chosen.
pub const DEFAULT_SAMPLE: u32 = 1000;
/// The choices offered in Settings.
pub const SAMPLE_SIZES: &[u32] = &[100, 500, 1000, 5000, 20000];
/// Documents with dynamic keys (maps keyed by id) would grow the list without
/// bound; past this many paths new ones are ignored.
const MAX_FIELDS: usize = 1000;
const MAX_DEPTH: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// Dotted path, as used in queries: `address.city`, `items.sku`.
    pub path: String,
    /// How many sampled documents have it.
    pub count: u64,
    /// `$type` names with how often each was seen, most common first.
    pub types: Vec<(&'static str, u64)>,
    /// For arrays: the element types, most common first.
    pub items: Vec<(&'static str, u64)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schema {
    pub sampled: u64,
    /// Sorted by path.
    pub fields: Vec<Field>,
    /// Some paths were left out because there were too many.
    pub truncated: bool,
}

impl Schema {
    pub fn field(&self, path: &str) -> Option<&Field> {
        self.fields
            .binary_search_by(|f| f.path.as_str().cmp(path))
            .ok()
            .map(|i| &self.fields[i])
    }
}

/// The `$type` alias of a value.
pub fn type_name(value: &Bson) -> &'static str {
    match value {
        Bson::Double(_) => "double",
        Bson::String(_) => "string",
        Bson::Document(_) => "object",
        Bson::Array(_) => "array",
        Bson::Binary(_) => "binData",
        Bson::Undefined => "undefined",
        Bson::ObjectId(_) => "objectId",
        Bson::Boolean(_) => "bool",
        Bson::DateTime(_) => "date",
        Bson::Null => "null",
        Bson::RegularExpression(_) => "regex",
        Bson::DbPointer(_) => "dbPointer",
        Bson::JavaScriptCode(_) => "javascript",
        Bson::Symbol(_) => "symbol",
        Bson::JavaScriptCodeWithScope(_) => "javascriptWithScope",
        Bson::Int32(_) => "int",
        Bson::Timestamp(_) => "timestamp",
        Bson::Int64(_) => "long",
        Bson::Decimal128(_) => "decimal",
        Bson::MinKey => "minKey",
        Bson::MaxKey => "maxKey",
    }
}

#[derive(Default)]
struct Acc {
    count: u64,
    /// The document that last counted, so a path seen twice in one document
    /// (inside an array) counts once.
    last_doc: u64,
    types: Vec<(&'static str, u64)>,
    items: Vec<(&'static str, u64)>,
}

fn bump(counts: &mut Vec<(&'static str, u64)>, name: &'static str) {
    match counts.iter_mut().find(|(n, _)| *n == name) {
        Some((_, c)) => *c += 1,
        None => counts.push((name, 1)),
    }
}

fn by_count(mut counts: Vec<(&'static str, u64)>) -> Vec<(&'static str, u64)> {
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    counts
}

#[derive(Default)]
pub struct Builder {
    sampled: u64,
    fields: HashMap<String, Acc>,
    truncated: bool,
}

impl Builder {
    pub fn add(&mut self, doc: &Document) {
        self.sampled += 1;
        self.walk(doc, "", 0);
    }

    fn walk(&mut self, doc: &Document, prefix: &str, depth: usize) {
        if depth >= MAX_DEPTH {
            return;
        }
        for (key, value) in doc {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            self.record(&path, value);
            match value {
                Bson::Document(sub) => self.walk(sub, &path, depth + 1),
                Bson::Array(items) => {
                    for item in items {
                        if let Bson::Document(sub) = item {
                            self.walk(sub, &path, depth + 1);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn record(&mut self, path: &str, value: &Bson) {
        let doc = self.sampled;
        if !self.fields.contains_key(path) {
            if self.fields.len() >= MAX_FIELDS {
                self.truncated = true;
                return;
            }
            self.fields.insert(path.to_string(), Acc::default());
        }
        let acc = self.fields.get_mut(path).expect("just inserted");
        if acc.last_doc != doc {
            acc.last_doc = doc;
            acc.count += 1;
        }
        bump(&mut acc.types, type_name(value));
        if let Bson::Array(items) = value {
            for item in items {
                bump(&mut acc.items, type_name(item));
            }
        }
    }

    pub fn finish(self) -> Schema {
        let mut fields: Vec<Field> = self
            .fields
            .into_iter()
            .map(|(path, acc)| Field {
                path,
                count: acc.count,
                types: by_count(acc.types),
                items: by_count(acc.items),
            })
            .collect();
        fields.sort_by(|a, b| a.path.cmp(&b.path));
        Schema {
            sampled: self.sampled,
            fields,
            truncated: self.truncated,
        }
    }
}

#[cfg(test)]
pub fn infer<'a>(docs: impl IntoIterator<Item = &'a Document>) -> Schema {
    let mut builder = Builder::default();
    for doc in docs {
        builder.add(doc);
    }
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    fn paths(schema: &Schema) -> Vec<&str> {
        schema.fields.iter().map(|f| f.path.as_str()).collect()
    }

    #[test]
    fn empty_sample_has_no_fields() {
        let schema = infer(&[]);
        assert_eq!(schema.sampled, 0);
        assert!(schema.fields.is_empty());
    }

    #[test]
    fn collects_nested_paths_sorted() {
        let docs = [doc! { "name": "a", "address": { "city": "Izmir", "zip": 35000 } }];
        let schema = infer(&docs);
        assert_eq!(paths(&schema), ["address", "address.city", "address.zip", "name"]);
        assert_eq!(schema.field("address").unwrap().types, [("object", 1)]);
        assert_eq!(schema.field("address.zip").unwrap().types, [("int", 1)]);
    }

    #[test]
    fn counts_how_many_documents_have_each_field() {
        let docs = [doc! { "a": 1, "b": 1 }, doc! { "a": 2 }, doc! { "a": "x" }];
        let schema = infer(&docs);
        assert_eq!(schema.sampled, 3);
        let a = schema.field("a").unwrap();
        assert_eq!(a.count, 3);
        assert_eq!(a.types, [("int", 2), ("string", 1)], "most common type first");
        assert_eq!(schema.field("b").unwrap().count, 1);
    }

    #[test]
    fn arrays_record_element_types_and_document_fields() {
        let docs = [doc! {
            "tags": ["a", "b"],
            "items": [{ "sku": "x", "qty": 1 }, { "sku": "y" }],
        }];
        let schema = infer(&docs);
        let tags = schema.field("tags").unwrap();
        assert_eq!(tags.types, [("array", 1)]);
        assert_eq!(tags.items, [("string", 2)]);
        assert_eq!(schema.field("items").unwrap().items, [("object", 2)]);
        let sku = schema.field("items.sku").unwrap();
        assert_eq!(sku.count, 1, "seen twice in one document, counted once");
        assert!(schema.field("items.qty").is_some());
    }

    #[test]
    fn caps_the_number_of_paths() {
        let mut map = Document::new();
        for i in 0..(MAX_FIELDS + 50) {
            map.insert(format!("k{i}"), 1);
        }
        let schema = infer(&[map]);
        assert_eq!(schema.fields.len(), MAX_FIELDS);
        assert!(schema.truncated);
        assert!(!infer(&[doc! { "a": 1 }]).truncated);
    }

    #[test]
    fn stops_at_a_maximum_depth() {
        let mut deep = doc! { "x": 1 };
        for _ in 0..(MAX_DEPTH + 5) {
            deep = doc! { "n": deep };
        }
        let schema = infer(&[deep]);
        let longest = schema.fields.iter().map(|f| f.path.split('.').count()).max().unwrap();
        assert_eq!(longest, MAX_DEPTH);
    }

    #[test]
    fn names_types_like_dollar_type() {
        assert_eq!(type_name(&Bson::Int64(1)), "long");
        assert_eq!(type_name(&Bson::Boolean(true)), "bool");
        assert_eq!(type_name(&Bson::DateTime(mongodb::bson::DateTime::now())), "date");
        assert_eq!(
            type_name(&Bson::ObjectId(mongodb::bson::oid::ObjectId::new())),
            "objectId"
        );
    }
}
