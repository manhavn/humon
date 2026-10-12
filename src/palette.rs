//! Quick open (⌘K): find any collection or a database's shell by typing.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Collection(String),
    Shell,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub conn_id: String,
    pub conn_name: String,
    pub db: String,
    pub target: Target,
}

impl Item {
    pub fn label(&self) -> String {
        match &self.target {
            Target::Collection(coll) => format!("{}.{coll}", self.db),
            Target::Shell => format!(">_ {}", self.db),
        }
    }
}

/// How well `text` matches `query` (both lowercase): exact, prefix,
/// contains, or its letters in order. Lower is better; None is no match.
fn tier(text: &str, query: &str) -> Option<u32> {
    if text == query {
        Some(0)
    } else if text.starts_with(query) {
        Some(1)
    } else if text.contains(query) {
        Some(2)
    } else {
        let mut chars = text.chars();
        query.chars().all(|q| chars.any(|c| c == q)).then_some(3)
    }
}

fn score(item: &Item, query: &str) -> Option<u32> {
    let db = item.db.to_lowercase();
    // ">_ name" asks for shells only.
    if let Some(rest) = query.strip_prefix(">_").or_else(|| query.strip_prefix('>')) {
        return match item.target {
            Target::Shell => tier(&db, rest.trim()),
            Target::Collection(_) => None,
        };
    }
    match &item.target {
        Target::Collection(coll) => {
            let coll = coll.to_lowercase();
            // "db.coll" narrows to a database.
            if let Some((db_q, coll_q)) = query.split_once('.') {
                if !db.starts_with(db_q) {
                    return None;
                }
                return if coll_q.is_empty() { Some(0) } else { tier(&coll, coll_q) };
            }
            tier(&coll, query).or_else(|| tier(&format!("{db}.{coll}"), query).map(|t| t + 10))
        }
        Target::Shell if query.contains('.') => None,
        Target::Shell => tier(&db, query).or_else(|| (query.len() >= 2 && "shell".starts_with(query)).then_some(1)),
    }
}

/// Database methods in the shell's prelude; `db.<name>` would call these.
const DB_METHODS: &[&str] = &["adminCommand", "createCollection", "dropDatabase", "getCollection", "getCollectionInfos", "getCollectionNames", "getName", "getSiblingDB", "runCommand", "serverStatus", "stats", "version"];

/// The command a shell opened from a collection starts with.
pub fn shell_starter(coll: &str) -> String {
    let mut chars = coll.chars();
    let identifier = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if identifier && !DB_METHODS.contains(&coll) && !coll.starts_with('_') {
        format!("db.{coll}.find()")
    } else {
        format!("db.getCollection({}).find()", serde_json::to_string(coll).unwrap_or_default())
    }
}

/// Indexes into `items` that match `query`, best first; ties keep their order.
pub fn rank(items: &[Item], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let mut scored: Vec<(u32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| if query.is_empty() { Some((0, i)) } else { score(item, &query).map(|s| (s, i)) })
        .collect();
    scored.sort_by_key(|&(s, _)| s);
    scored.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::{DB_METHODS, Item, Target, rank, shell_starter};

    fn coll(db: &str, coll: &str) -> Item {
        Item { conn_id: "c".into(), conn_name: "Local".into(), db: db.into(), target: Target::Collection(coll.into()) }
    }

    fn shell(db: &str) -> Item {
        Item { conn_id: "c".into(), conn_name: "Local".into(), db: db.into(), target: Target::Shell }
    }

    fn labels(items: &[Item], query: &str) -> Vec<String> {
        rank(items, query).into_iter().map(|i| items[i].label()).collect()
    }

    #[test]
    fn best_matches_come_first() {
        let items = vec![coll("shop", "orders"), coll("shop", "users"), coll("logs", "user_events"), coll("shop", "superusers")];
        assert_eq!(labels(&items, "users"), ["shop.users", "shop.superusers", "logs.user_events"], "exact, contains, then letters in order");
        assert_eq!(labels(&items, "user"), ["shop.users", "logs.user_events", "shop.superusers"], "prefix beats contains");
        assert_eq!(labels(&items, "USR"), ["shop.users", "logs.user_events", "shop.superusers"], "letters in order, any case");
        assert!(labels(&items, "zzz").is_empty());
    }

    #[test]
    fn db_dot_collection_narrows_by_database() {
        let items = vec![coll("shop", "users"), coll("logs", "users")];
        assert_eq!(labels(&items, "logs.us"), ["logs.users"]);
        assert_eq!(labels(&items, "shop."), ["shop.users"]);
    }

    #[test]
    fn shells_match_their_database_or_the_word_shell() {
        let items = vec![coll("shop", "users"), shell("shop"), shell("logs")];
        assert_eq!(labels(&items, "shell"), [">_ shop", ">_ logs"]);
        assert_eq!(labels(&items, ">_ lo"), [">_ logs"]);
        assert_eq!(labels(&items, "shop"), [">_ shop", "shop.users"], "a database's shell before its collections");
    }

    #[test]
    fn shell_starts_with_a_find_on_the_collection() {
        assert_eq!(shell_starter("users"), "db.users.find()");
        assert_eq!(shell_starter("my-coll"), r#"db.getCollection("my-coll").find()"#);
        assert_eq!(shell_starter("2024"), r#"db.getCollection("2024").find()"#);
        // Names that are Database methods would call the method instead.
        assert_eq!(shell_starter("stats"), r#"db.getCollection("stats").find()"#);
        assert_eq!(shell_starter("getName"), r#"db.getCollection("getName").find()"#);
    }

    #[test]
    fn db_methods_list_matches_the_prelude() {
        let js = include_str!("console/prelude.js");
        let start = js.find("Object.assign(Database.prototype, {").unwrap();
        let end = js[start..].find("\n  });").unwrap() + start;
        let mut methods: Vec<&str> = js[start..end]
            .lines()
            .filter_map(|l| l.strip_prefix("    ").and_then(|l| l.split_once('(')).map(|(name, _)| name))
            .filter(|name| name.chars().all(|c| c.is_ascii_alphanumeric()))
            .collect();
        methods.sort_unstable();
        assert_eq!(methods, DB_METHODS);
    }

    #[test]
    fn empty_query_lists_everything_in_order() {
        let items = vec![coll("shop", "users"), shell("shop")];
        assert_eq!(labels(&items, "  "), ["shop.users", ">_ shop"]);
    }
}
