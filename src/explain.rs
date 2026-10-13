//! What `explain` says about a query, in a few numbers: the plan's stages,
//! the indexes it used, and how many keys and documents it read to return
//! how many.

use mongodb::bson::{Bson, Document};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    /// The winning plan's stages, outermost first, down its first input.
    pub stages: Vec<String>,
    /// Every index the plan reads.
    pub indexes: Vec<String>,
    /// The plan reads the whole collection.
    pub collection_scan: bool,
    pub returned: i64,
    pub keys_examined: i64,
    pub docs_examined: i64,
    pub millis: i64,
}

/// What a plan did, in one line.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Read the whole collection (on at least one shard).
    CollectionScan,
    /// Read through these indexes.
    Index(Vec<String>),
    /// Neither, such as `IDHACK` for an `_id` lookup: the plan's first stage.
    Plan(String),
    /// The explain held no plan to read.
    Unknown,
}

impl Summary {
    pub fn verdict(&self) -> Verdict {
        if self.collection_scan {
            Verdict::CollectionScan
        } else if !self.indexes.is_empty() {
            Verdict::Index(self.indexes.clone())
        } else if let Some(stage) = self.stages.first() {
            Verdict::Plan(stage.clone())
        } else {
            Verdict::Unknown
        }
    }
}

fn number(doc: &Document, key: &str) -> i64 {
    match doc.get(key) {
        Some(Bson::Int32(n)) => *n as i64,
        Some(Bson::Int64(n)) => *n,
        Some(Bson::Double(n)) => *n as i64,
        _ => 0,
    }
}

/// Walks a plan stage: its name, then its input (or each of its inputs).
fn walk(stage: &Document, path: &mut Vec<String>, first: bool, out: &mut Summary) {
    let name = stage.get_str("stage").unwrap_or_default().to_string();
    if name == "COLLSCAN" {
        out.collection_scan = true;
    }
    if let Ok(index) = stage.get_str("indexName")
        && !out.indexes.iter().any(|i| i == index)
    {
        out.indexes.push(index.to_string());
    }
    if first {
        path.push(name);
    }
    if let Ok(input) = stage.get_document("inputStage") {
        walk(input, path, first, out);
    }
    // A sharded plan: each shard has a plan of its own.
    if let Ok(shards) = stage.get_array("shards") {
        for (i, shard) in shards.iter().enumerate() {
            if let Some(plan) = shard.as_document().and_then(|d| d.get_document("winningPlan").ok()) {
                let plan = plan.get_document("queryPlan").unwrap_or(plan);
                walk(plan, path, first && i == 0, out);
            }
        }
    }
    if let Ok(inputs) = stage.get_array("inputStages") {
        for (i, input) in inputs.iter().enumerate() {
            if let Bson::Document(input) = input {
                walk(input, path, first && i == 0, out);
            }
        }
    }
}

/// The few numbers that matter from an `executionStats` explain.
pub fn summarize(explain: &Document) -> Summary {
    let mut out = Summary::default();
    let plan = explain
        .get_document("queryPlanner")
        .and_then(|p| p.get_document("winningPlan"))
        .ok()
        .map(|w| w.get_document("queryPlan").unwrap_or(w));
    if let Some(plan) = plan {
        let mut stages = Vec::new();
        walk(plan, &mut stages, true, &mut out);
        out.stages = stages;
    }
    if let Ok(stats) = explain.get_document("executionStats") {
        out.returned = number(stats, "nReturned");
        out.keys_examined = number(stats, "totalKeysExamined");
        out.docs_examined = number(stats, "totalDocsExamined");
        out.millis = number(stats, "executionTimeMillis");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn an_index_scan() {
        let explain = doc! {
            "queryPlanner": { "winningPlan": {
                "stage": "FETCH",
                "inputStage": { "stage": "IXSCAN", "indexName": "age_1", "keyPattern": { "age": 1 } },
            } },
            "executionStats": {
                "nReturned": 12, "executionTimeMillis": 3, "totalKeysExamined": 12, "totalDocsExamined": 12,
            },
        };
        let s = summarize(&explain);
        assert_eq!(s.stages, ["FETCH", "IXSCAN"]);
        assert_eq!(s.indexes, ["age_1"]);
        assert!(!s.collection_scan);
        assert_eq!(
            (s.returned, s.keys_examined, s.docs_examined, s.millis),
            (12, 12, 12, 3)
        );
    }

    #[test]
    fn a_collection_scan() {
        let explain = doc! {
            "queryPlanner": { "winningPlan": { "stage": "COLLSCAN" } },
            "executionStats": { "nReturned": 1, "executionTimeMillis": 40, "totalKeysExamined": 0, "totalDocsExamined": 137 },
        };
        let s = summarize(&explain);
        assert!(s.collection_scan);
        assert!(s.indexes.is_empty());
        assert_eq!(s.docs_examined, 137);
    }

    #[test]
    fn newer_servers_nest_the_plan_and_or_has_several_inputs() {
        let explain = doc! {
            "queryPlanner": { "winningPlan": { "queryPlan": {
                "stage": "SUBPLAN",
                "inputStage": { "stage": "OR", "inputStages": [
                    { "stage": "IXSCAN", "indexName": "a_1" },
                    { "stage": "IXSCAN", "indexName": "b_1" },
                ] },
            } } },
            "executionStats": { "nReturned": 2, "executionTimeMillis": 0, "totalKeysExamined": 2, "totalDocsExamined": 2 },
        };
        let s = summarize(&explain);
        assert_eq!(s.stages, ["SUBPLAN", "OR", "IXSCAN"]);
        assert_eq!(s.indexes, ["a_1", "b_1"]);
    }

    #[test]
    fn numbers_may_come_as_any_integer_type() {
        let explain = doc! {
            "queryPlanner": { "winningPlan": { "stage": "COLLSCAN" } },
            "executionStats": { "nReturned": 5_i64, "executionTimeMillis": 1.0, "totalDocsExamined": 9_i64 },
        };
        let s = summarize(&explain);
        assert_eq!((s.returned, s.millis, s.docs_examined, s.keys_examined), (5, 1, 9, 0));
    }

    #[test]
    fn sharded_plans_are_read_shard_by_shard() {
        let explain = doc! {
            "queryPlanner": { "winningPlan": { "stage": "SHARD_MERGE", "shards": [
                { "shardName": "a", "winningPlan": { "stage": "COLLSCAN" } },
                { "shardName": "b", "winningPlan": { "stage": "FETCH", "inputStage": { "stage": "IXSCAN", "indexName": "x_1" } } },
            ] } },
        };
        let s = summarize(&explain);
        assert!(s.collection_scan, "one shard reads everything");
        assert_eq!(s.indexes, ["x_1"]);
        assert_eq!(s.stages, ["SHARD_MERGE", "COLLSCAN"]);
    }

    #[test]
    fn the_verdict_names_what_happened() {
        let plan = |stage: &str| doc! { "queryPlanner": { "winningPlan": { "stage": stage } } };
        assert_eq!(summarize(&plan("COLLSCAN")).verdict(), Verdict::CollectionScan);
        assert_eq!(summarize(&plan("IDHACK")).verdict(), Verdict::Plan("IDHACK".into()));
        assert_eq!(summarize(&doc! {}).verdict(), Verdict::Unknown);
        let indexed = doc! { "queryPlanner": { "winningPlan": { "stage": "FETCH", "inputStage": { "stage": "IXSCAN", "indexName": "a_1" } } } };
        assert_eq!(summarize(&indexed).verdict(), Verdict::Index(vec!["a_1".into()]));
    }
}
