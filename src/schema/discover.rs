//! Finding the fields a random sample missed (issue #3): the schema becomes a
//! strict `$jsonSchema`, the server is asked for documents that don't match
//! it, and those are folded in until none are left or the budget runs out.

use mongodb::bson::{Document, doc};

use super::{Field, MAX_DEPTH, Schema};

impl Schema {
    /// A `$jsonSchema` that only these fields, with these types, pass: every
    /// object level forbids other fields, and arrays their unseen element types.
    pub fn json_schema(&self) -> Document {
        doc! {
            "bsonType": "object",
            "properties": self.properties(""),
            "additionalProperties": false,
        }
    }

    /// The fields directly under `prefix` ("" for the top level, else "a.").
    fn properties(&self, prefix: &str) -> Document {
        self.fields
            .iter()
            .filter(|f| f.path.strip_prefix(prefix).is_some_and(|rest| !rest.contains('.')))
            .map(|f| (f.path[prefix.len()..].to_string(), self.field_schema(f).into()))
            .collect()
    }

    fn field_schema(&self, field: &Field) -> Document {
        let names = |counts: &[(&'static str, u64)]| {
            let mut names: Vec<&str> = counts.iter().map(|(t, _)| *t).collect();
            names.sort_unstable();
            names
        };
        let has = |counts: &[(&'static str, u64)], name: &str| counts.iter().any(|(t, _)| *t == name);
        // Below the depth limit nothing was recorded, so it can't be required absent.
        let open = field.path.split('.').count() >= MAX_DEPTH;
        let children = format!("{}.", field.path);
        let mut out = doc! { "bsonType": names(&field.types) };
        if has(&field.types, "object") && !open {
            out.insert("properties", self.properties(&children));
            out.insert("additionalProperties", false);
        }
        if has(&field.types, "array") {
            if field.items.is_empty() {
                out.insert("maxItems", 0);
            } else {
                let mut items = doc! { "bsonType": names(&field.items) };
                if has(&field.items, "object") && !open {
                    items.insert("properties", self.properties(&children));
                    items.insert("additionalProperties", false);
                }
                out.insert("items", items);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::schema::{Builder, infer};
    use mongodb::bson::{Bson, doc};

    #[test]
    fn json_schema_is_strict_at_every_level() {
        let schema = infer(&[doc! {
            "_id": 1, "name": "a", "address": { "city": "Izmir" },
        }]);
        assert_eq!(
            schema.json_schema(),
            doc! {
                "bsonType": "object",
                "properties": {
                    "_id": { "bsonType": ["int"] },
                    "address": {
                        "bsonType": ["object"],
                        "properties": { "city": { "bsonType": ["string"] } },
                        "additionalProperties": false,
                    },
                    "name": { "bsonType": ["string"] },
                },
                "additionalProperties": false,
            }
        );
    }

    #[test]
    fn every_seen_type_is_allowed() {
        let schema = infer(&[doc! { "a": 1 }, doc! { "a": "x" }, doc! { "a": Bson::Null }]);
        let types = schema
            .json_schema()
            .get_document("properties")
            .unwrap()
            .get_document("a")
            .unwrap()
            .clone();
        assert_eq!(types, doc! { "bsonType": ["int", "null", "string"] });
    }

    #[test]
    fn arrays_constrain_their_elements() {
        let schema = infer(&[doc! { "tags": ["x"], "items": [{ "sku": "s" }], "empty": [] }]);
        let props = schema.json_schema().get_document("properties").unwrap().clone();
        assert_eq!(
            props.get_document("tags").unwrap(),
            &doc! { "bsonType": ["array"], "items": { "bsonType": ["string"] } }
        );
        assert_eq!(
            props.get_document("items").unwrap(),
            &doc! {
                "bsonType": ["array"],
                "items": {
                    "bsonType": ["object"],
                    "properties": { "sku": { "bsonType": ["string"] } },
                    "additionalProperties": false,
                },
            }
        );
        assert_eq!(
            props.get_document("empty").unwrap(),
            &doc! { "bsonType": ["array"], "maxItems": 0 },
            "only empty arrays seen: any element is new"
        );
    }

    #[test]
    fn objects_at_the_depth_limit_stay_open() {
        // Fields below the depth limit are never recorded, so requiring their
        // absence would make every deep document a counterexample forever.
        let mut deep = doc! { "x": 1 };
        for _ in 0..20 {
            deep = doc! { "n": deep };
        }
        let schema = infer(&[deep]);
        let mut level = schema.json_schema();
        let mut depth = 0;
        while let Ok(props) = level.get_document("properties") {
            level = props.get_document("n").unwrap().clone();
            depth += 1;
        }
        assert!(!level.contains_key("additionalProperties"), "open at depth {depth}");
    }

    #[test]
    fn found_documents_add_fields_without_counting_as_sampled() {
        let mut builder = Builder::default();
        builder.add(&doc! { "a": 1 });
        builder.add(&doc! { "a": 2 });
        builder.add_found(&doc! { "a": "x", "rare": true });
        let schema = builder.snapshot();
        assert_eq!(schema.sampled, 2, "found documents aren't a random sample");
        let a = schema.field("a").unwrap();
        assert_eq!(a.count, 2);
        assert!(a.types.iter().any(|(t, _)| *t == "string"), "the new type is known");
        assert_eq!(
            schema.field("rare").unwrap().count,
            0,
            "known, but not seen in the sample"
        );
        assert_eq!(builder.snapshot(), schema, "a snapshot leaves the builder as it was");
    }

    #[test]
    fn a_fresh_schema_is_not_known_complete() {
        assert!(!infer(&[doc! { "a": 1 }]).complete);
    }
}
