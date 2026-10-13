//! How aggregation stages change the shape of documents, so completion
//! after `$project` or `$group` offers the fields that exist there.
//!
//! The specifications say what each stage accepts, not what it outputs; this
//! covers the stages that reshape documents. Any other stage leaves the
//! schema as it is.

use mongodb::bson::{Bson, Document};

use super::{Field, Schema};
use crate::mql;

impl Schema {
    /// The schema after running `stages` on documents of this one.
    pub fn after(&self, stages: &[Document]) -> Schema {
        stages.iter().fold(self.clone(), |schema, stage| schema.apply(stage))
    }

    fn apply(mut self, stage: &Document) -> Schema {
        let Some((name, spec)) = stage.iter().next() else {
            return self;
        };
        match (name.as_str(), spec) {
            ("$project", Bson::Document(spec)) => self.project(spec),
            ("$addFields" | "$set", Bson::Document(spec)) => {
                for (path, value) in flatten(spec, "") {
                    let types = expr_types(value, &self);
                    self.put(&path, types);
                }
                self
            }
            ("$unset", value) => {
                for path in strings(value) {
                    self.remove(&path);
                }
                self
            }
            ("$group", Bson::Document(spec)) => {
                let mut out = self.empty();
                for (key, value) in spec {
                    let types = if key == "_id" {
                        expr_types(value, &self)
                    } else {
                        accumulator_types(value, &self)
                    };
                    out.put(key, types);
                }
                out
            }
            ("$unwind", spec) => {
                let (path, index) = match spec {
                    Bson::Document(d) => (
                        d.get_str("path").unwrap_or_default(),
                        d.get_str("includeArrayIndex").ok(),
                    ),
                    Bson::String(s) => (s.as_str(), None),
                    _ => ("", None),
                };
                if let Some(field) = path.strip_prefix('$').and_then(|p| self.field_mut(p))
                    && !field.items.is_empty()
                {
                    field.types = std::mem::take(&mut field.items);
                }
                if let Some(index) = index {
                    self.put(index, vec!["long"]);
                }
                self
            }
            ("$lookup", Bson::Document(spec)) => {
                if let Ok(name) = spec.get_str("as") {
                    self.remove(name);
                    self.put(name, vec!["array"]);
                    if let Some(field) = self.field_mut(name) {
                        field.items = vec![("object", 1)];
                    }
                }
                self
            }
            ("$count", Bson::String(name)) => {
                let mut out = self.empty();
                out.put(name, vec!["int"]);
                out
            }
            ("$sortByCount", value) => {
                let mut out = self.empty();
                out.put("_id", expr_types(value, &self));
                out.put("count", vec!["int"]);
                out
            }
            ("$bucket" | "$bucketAuto", Bson::Document(spec)) => {
                let mut out = self.empty();
                out.put("_id", vec![]);
                match spec.get_document("output") {
                    Ok(output) => {
                        for (key, value) in output {
                            out.put(key, accumulator_types(value, &self));
                        }
                    }
                    Err(_) => out.put("count", vec!["int"]),
                }
                out
            }
            ("$facet", Bson::Document(spec)) => {
                let mut out = self.empty();
                for key in spec.keys() {
                    out.put(key, vec!["array"]);
                }
                out
            }
            ("$replaceRoot", Bson::Document(spec)) => match spec.get("newRoot") {
                Some(root) => self.replace_root(root),
                None => self,
            },
            ("$replaceWith", root) => self.replace_root(root),
            _ => self,
        }
    }

    fn project(mut self, spec: &Document) -> Schema {
        let entries = flatten(spec, "");
        let excluded = |v: &Bson| matches!(v, Bson::Boolean(false)) || as_number(v) == Some(0.0);
        let included = |v: &Bson| matches!(v, Bson::Boolean(true)) || as_number(v).is_some_and(|n| n != 0.0);
        let inclusion = entries.iter().any(|(path, v)| path != "_id" && !excluded(v));
        if !inclusion {
            for (path, _) in &entries {
                self.remove(path);
            }
            return self;
        }
        let drop_id = entries.iter().any(|(path, v)| path == "_id" && excluded(v));
        let mut keep: Vec<&str> = entries
            .iter()
            .filter(|(_, v)| included(v))
            .map(|(path, _)| path.as_str())
            .collect();
        if !drop_id {
            keep.push("_id");
        }
        let mut out = self.empty();
        out.fields = self
            .fields
            .iter()
            .filter(|f| {
                keep.iter().any(|k| {
                    f.path == *k || f.path.starts_with(&format!("{k}.")) || k.starts_with(&format!("{}.", f.path))
                })
            })
            .cloned()
            .collect();
        for (path, value) in &entries {
            if !included(value) && !excluded(value) {
                let types = expr_types(value, &self);
                out.put(path, types);
            }
        }
        out
    }

    fn replace_root(self, root: &Bson) -> Schema {
        let mut out = self.empty();
        match root {
            Bson::String(s) if s.starts_with('$') && !s.starts_with("$$") => {
                let prefix = format!("{}.", &s[1..]);
                out.fields = self
                    .fields
                    .iter()
                    .filter_map(|f| {
                        f.path.strip_prefix(&prefix).map(|rest| Field {
                            path: rest.to_string(),
                            ..f.clone()
                        })
                    })
                    .collect();
            }
            Bson::Document(spec) if !is_operator(spec) => {
                for (path, value) in flatten(spec, "") {
                    out.put(&path, expr_types(value, &self));
                }
            }
            _ => {}
        }
        out
    }

    /// Same sample size, no fields: the start of a reshaped schema.
    fn empty(&self) -> Schema {
        Schema {
            sampled: self.sampled,
            fields: Vec::new(),
            truncated: false,
        }
    }

    fn field_mut(&mut self, path: &str) -> Option<&mut Field> {
        self.fields.iter_mut().find(|f| f.path == path)
    }

    /// Removes a field and everything under it.
    fn remove(&mut self, path: &str) {
        let under = format!("{path}.");
        self.fields.retain(|f| f.path != path && !f.path.starts_with(&under));
    }

    /// Sets a computed field, present in every document.
    fn put(&mut self, path: &str, types: Vec<&'static str>) {
        if !types.contains(&"object") {
            self.remove(path);
        }
        let field = Field {
            path: path.to_string(),
            count: self.sampled,
            types: types.into_iter().map(|t| (t, self.sampled)).collect(),
            items: Vec::new(),
        };
        match self.fields.binary_search_by(|f| f.path.as_str().cmp(path)) {
            Ok(i) => self.fields[i] = field,
            Err(i) => self.fields.insert(i, field),
        }
    }
}

fn as_number(v: &Bson) -> Option<f64> {
    match v {
        Bson::Int32(n) => Some(*n as f64),
        Bson::Int64(n) => Some(*n as f64),
        Bson::Double(n) => Some(*n),
        _ => None,
    }
}

fn is_operator(doc: &Document) -> bool {
    doc.keys().next().is_some_and(|k| k.starts_with('$'))
}

/// Field specs with nested documents spelled out as dotted paths:
/// `{ a: { b: 1 } }` is `a.b: 1`; operator documents are values.
fn flatten<'a>(spec: &'a Document, prefix: &str) -> Vec<(String, &'a Bson)> {
    let mut out = Vec::new();
    for (key, value) in spec {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            Bson::Document(sub) if !sub.is_empty() && !is_operator(sub) => out.extend(flatten(sub, &path)),
            _ => out.push((path, value)),
        }
    }
    out
}

fn strings(value: &Bson) -> Vec<String> {
    match value {
        Bson::String(s) => vec![s.clone()],
        Bson::Array(items) => items.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    }
}

/// `$type` names a spec type stands for.
fn spec_type(name: &str) -> Option<&'static str> {
    Some(match name {
        "int" => "int",
        "long" => "long",
        "double" | "number" => "double",
        "decimal" => "decimal",
        "string" => "string",
        "bool" => "bool",
        "date" => "date",
        "object" => "object",
        "array" => "array",
        "objectId" => "objectId",
        "binData" => "binData",
        "null" => "null",
        "timestamp" => "timestamp",
        "regex" => "regex",
        _ => return None,
    })
}

/// The types an expression can produce: a field reference has its field's
/// types, an operator the types the specification says it returns.
pub fn expr_types(value: &Bson, schema: &Schema) -> Vec<&'static str> {
    match value {
        Bson::String(s) if s.starts_with("$$") => Vec::new(),
        Bson::String(s) if s.starts_with('$') => schema
            .field(&s[1..])
            .map(|f| f.types.iter().map(|(t, _)| *t).collect())
            .unwrap_or_default(),
        Bson::Document(doc) if is_operator(doc) => {
            let name = doc.keys().next().map(String::as_str).unwrap_or_default();
            mql::find(name, mql::Kind::EXPRESSION)
                .map(|op| {
                    let mut types: Vec<&'static str> = op.returns.iter().filter_map(|r| spec_type(r)).collect();
                    types.dedup();
                    types
                })
                .unwrap_or_default()
        }
        Bson::Document(_) => vec!["object"],
        other => vec![super::type_name(other)],
    }
}

fn accumulator_types(value: &Bson, schema: &Schema) -> Vec<&'static str> {
    let Bson::Document(doc) = value else {
        return Vec::new();
    };
    let Some((name, arg)) = doc.iter().next() else {
        return Vec::new();
    };
    match name.as_str() {
        "$count" => vec!["int"],
        "$avg" | "$stdDevPop" | "$stdDevSamp" => vec!["double"],
        "$sum" => {
            let types = expr_types(arg, schema);
            if types.is_empty() || types.iter().any(|t| !["int", "long", "double", "decimal"].contains(t)) {
                vec!["int"]
            } else {
                types
            }
        }
        "$push" | "$addToSet" => vec!["array"],
        "$first" | "$last" | "$min" | "$max" => expr_types(arg, schema),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::infer;
    use mongodb::bson::doc;

    fn users() -> Schema {
        infer(&[doc! {
            "_id": 1, "name": "a", "age": 30, "tags": ["x"],
            "address": { "city": "Izmir", "zip": 35000 },
            "items": [{ "sku": "s1", "qty": 2 }],
        }])
    }

    fn paths(schema: &Schema) -> Vec<&str> {
        schema.fields.iter().map(|f| f.path.as_str()).collect()
    }

    fn types<'a>(schema: &'a Schema, path: &str) -> Vec<&'a str> {
        schema.field(path).unwrap().types.iter().map(|(t, _)| *t).collect()
    }

    #[test]
    fn filtering_stages_keep_the_shape() {
        let s = users();
        let after = s.after(&[
            doc! { "$match": { "age": 1 } },
            doc! { "$sort": { "age": 1 } },
            doc! { "$limit": 5 },
        ]);
        assert_eq!(after, s);
    }

    #[test]
    fn project_inclusion_keeps_listed_fields_and_id() {
        let after = users().after(&[doc! { "$project": { "name": 1, "address.city": 1 } }]);
        assert_eq!(paths(&after), ["_id", "address", "address.city", "name"]);
        let after = users().after(&[doc! { "$project": { "_id": 0, "address": { "zip": 1 } } }]);
        assert_eq!(paths(&after), ["address", "address.zip"]);
    }

    #[test]
    fn project_exclusion_removes_fields_and_their_subfields() {
        let after = users().after(&[doc! { "$project": { "address": 0, "tags": 0 } }]);
        assert_eq!(paths(&after), ["_id", "age", "items", "items.qty", "items.sku", "name"]);
    }

    #[test]
    fn computed_fields_get_types_from_the_expression() {
        let after = users().after(&[doc! { "$project": {
            "upper": { "$toUpper": "$name" }, "copy": "$age", "n": 1,
        } }]);
        assert_eq!(types(&after, "upper"), ["string"]);
        assert_eq!(types(&after, "copy"), ["int"]);
        assert!(after.field("name").is_none(), "only listed fields are kept");
        let after = users().after(&[doc! { "$addFields": { "first": { "$arrayElemAt": ["$tags", 0] }, "age": "x" } }]);
        assert!(after.field("first").is_some());
        assert_eq!(types(&after, "age"), ["string"], "replaces the old field");
        assert!(after.field("name").is_some(), "keeps the rest");
    }

    #[test]
    fn group_outputs_the_key_and_accumulators() {
        let after = users().after(&[doc! { "$group": {
            "_id": "$address.city", "n": { "$sum": 1 }, "avg": { "$avg": "$age" },
            "names": { "$push": "$name" }, "oldest": { "$max": "$age" },
        } }]);
        assert_eq!(paths(&after), ["_id", "avg", "n", "names", "oldest"]);
        assert_eq!(types(&after, "_id"), ["string"]);
        assert_eq!(types(&after, "n"), ["int"]);
        assert_eq!(types(&after, "avg"), ["double"]);
        assert_eq!(types(&after, "names"), ["array"]);
        assert_eq!(types(&after, "oldest"), ["int"]);
    }

    #[test]
    fn unwind_turns_an_array_into_its_elements() {
        let after = users().after(&[doc! { "$unwind": "$items" }]);
        assert_eq!(types(&after, "items"), ["object"]);
        assert!(after.field("items.sku").is_some());
        let after = users().after(&[doc! { "$unwind": { "path": "$tags", "includeArrayIndex": "i" } }]);
        assert_eq!(types(&after, "tags"), ["string"]);
        assert_eq!(types(&after, "i"), ["long"]);
    }

    #[test]
    fn reshaping_stages() {
        let s = users();
        assert_eq!(paths(&s.after(&[doc! { "$count": "total" }])), ["total"]);
        assert_eq!(paths(&s.after(&[doc! { "$sortByCount": "$name" }])), ["_id", "count"]);
        let after = s.after(&[
            doc! { "$lookup": { "from": "orders", "localField": "_id", "foreignField": "u", "as": "orders" } },
        ]);
        assert_eq!(types(&after, "orders"), ["array"]);
        let after = s.after(&[doc! { "$replaceRoot": { "newRoot": "$address" } }]);
        assert_eq!(paths(&after), ["city", "zip"]);
        assert_eq!(paths(&s.after(&[doc! { "$unset": ["address", "tags"] }])).len(), 6);
        assert_eq!(paths(&s.after(&[doc! { "$facet": { "a": [], "b": [] } }])), ["a", "b"]);
    }

    #[test]
    fn stages_chain() {
        let after = users().after(&[
            doc! { "$unwind": "$items" },
            doc! { "$group": { "_id": "$items.sku", "qty": { "$sum": "$items.qty" } } },
        ]);
        assert_eq!(paths(&after), ["_id", "qty"]);
        assert_eq!(types(&after, "qty"), ["int"]);
    }
}
