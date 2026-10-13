//! The schema as rows for the Schema view: one per field, nested under its
//! parent, with how often it appears and the share of each type.

use super::Schema;

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub path: String,
    /// The last part of the path.
    pub name: String,
    pub depth: usize,
    /// Percent of sampled documents that have it; None when it was only found
    /// by searching the collection, so it is in under 1% of a sample.
    pub presence: Option<u8>,
    /// Type names with their share of the field's values, most common first.
    pub types: Vec<(String, u8)>,
    /// For arrays: the element types, the same way.
    pub items: Vec<(String, u8)>,
}

fn shares(counts: &[(&'static str, u64)]) -> Vec<(String, u8)> {
    let total: u64 = counts.iter().map(|(_, n)| n).sum();
    counts
        .iter()
        .map(|(t, n)| {
            (
                t.to_string(),
                (n * 100 + total / 2).checked_div(total).unwrap_or(0) as u8,
            )
        })
        .collect()
}

impl Schema {
    /// One row per field, each right under its parent.
    pub fn overview(&self) -> Vec<Row> {
        let mut fields: Vec<_> = self.fields.iter().collect();
        fields.sort_by(|a, b| a.path.split('.').cmp(b.path.split('.')));
        fields
            .into_iter()
            .map(|f| Row {
                path: f.path.clone(),
                name: f.path.rsplit('.').next().unwrap_or_default().to_string(),
                depth: f.path.matches('.').count(),
                presence: match (f.count, self.sampled) {
                    (0, _) | (_, 0) => None,
                    (n, total) => Some(((n * 100).div_ceil(total)).min(100) as u8),
                },
                types: shares(&f.types),
                items: shares(&f.items),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::schema::{Builder, infer};
    use mongodb::bson::{Bson, doc};

    #[test]
    fn rows_follow_the_tree_with_presence_and_type_shares() {
        let schema = infer(&[
            doc! { "name": "a", "age": 1, "address": { "city": "x" }, "tags": ["t", 1] },
            doc! { "name": "b", "age": "old" },
            doc! { "name": "c", "age": 3 },
            doc! { "name": "d", "age": Bson::Null },
        ]);
        let rows = schema.overview();
        let names: Vec<(&str, usize)> = rows.iter().map(|r| (r.name.as_str(), r.depth)).collect();
        assert_eq!(
            names,
            [("address", 0), ("city", 1), ("age", 0), ("name", 0), ("tags", 0)]
        );
        let age = rows.iter().find(|r| r.path == "age").unwrap();
        assert_eq!(age.presence, Some(100));
        assert_eq!(
            age.types,
            [("int".into(), 50), ("null".into(), 25), ("string".into(), 25)]
        );
        let city = rows.iter().find(|r| r.path == "address.city").unwrap();
        assert_eq!(city.presence, Some(25));
        let tags = rows.iter().find(|r| r.path == "tags").unwrap();
        assert_eq!(
            tags.items,
            [("int".into(), 50), ("string".into(), 50)],
            "array element types"
        );
    }

    #[test]
    fn fields_found_only_by_searching_have_no_presence() {
        let mut builder = Builder::default();
        builder.add(&doc! { "a": 1 });
        builder.add_found(&doc! { "rare": 1 });
        let rows = builder.finish().overview();
        assert_eq!(rows.iter().find(|r| r.path == "rare").unwrap().presence, None);
    }
}
