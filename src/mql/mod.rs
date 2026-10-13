//! Every MQL operator, stage, expression and accumulator, generated from
//! MongoDB's own specifications (see `scripts/gen-mql.rb`). Completion lists
//! come from here, and argument types feed validation.

mod spec;

pub use spec::OPS;

/// What an operator is, as bit flags: some are more than one thing
/// (`$sum` is an accumulator and a window operator).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Kind(pub u16);

impl Kind {
    /// Goes under a field: `{ age: { $gt: 1 } }`.
    pub const FIELD_QUERY: Kind = Kind(1);
    /// Goes at the top of a filter: `$and`, `$expr`.
    pub const QUERY: Kind = Kind(1 << 1);
    pub const GEOMETRY: Kind = Kind(1 << 2);
    pub const STAGE: Kind = Kind(1 << 3);
    /// A stage allowed in an update pipeline.
    pub const UPDATE_STAGE: Kind = Kind(1 << 4);
    pub const EXPRESSION: Kind = Kind(1 << 5);
    pub const ACCUMULATOR: Kind = Kind(1 << 6);
    pub const WINDOW: Kind = Kind(1 << 7);
    pub const UPDATE: Kind = Kind(1 << 8);

    pub fn has(self, other: Kind) -> bool {
        self.0 & other.0 != 0
    }
}

/// How the operator's arguments are written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Encode {
    /// One value: `{ $gt: 5 }`, or an array for several arguments.
    Single,
    /// Named arguments: `{ $unwind: { path: "$a" } }`.
    Object,
    /// Positional arguments: `{ $subtract: [a, b] }`.
    Array,
}

#[derive(Debug)]
pub struct Arg {
    pub name: &'static str,
    /// Spec type names: "int", "string", "expression", "resolvesToNumber", ...
    pub types: &'static [&'static str],
    pub optional: bool,
    /// "array" or "object" when the argument repeats.
    pub variadic: Option<&'static str>,
}

#[derive(Debug)]
pub struct Op {
    pub name: &'static str,
    pub kind: Kind,
    pub encode: Encode,
    /// The first sentence of the description.
    pub summary: &'static str,
    pub link: &'static str,
    /// For expressions: the types it can produce ("int", "string", ...).
    pub returns: &'static [&'static str],
    pub args: &'static [Arg],
}

/// Operators of a kind, in alphabetical order.
pub fn of(kind: Kind) -> impl Iterator<Item = &'static Op> {
    OPS.iter().filter(move |op| op.kind.has(kind))
}

/// An operator by name and kind; `$and` exists both as a query and an expression.
pub fn find(name: &str, kind: Kind) -> Option<&'static Op> {
    of(kind).find(|op| op.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_every_family() {
        assert!(of(Kind::FIELD_QUERY).count() >= 20);
        assert!(of(Kind::STAGE).count() >= 30);
        assert!(of(Kind::EXPRESSION).count() >= 150);
        assert!(of(Kind::ACCUMULATOR).count() >= 15);
        assert!(of(Kind::UPDATE).count() >= 10);
    }

    #[test]
    fn finds_operators_by_kind() {
        let gt = find("$gt", Kind::FIELD_QUERY).unwrap();
        assert!(gt.summary.starts_with("Matches values that are greater than"));
        assert!(gt.link.starts_with("https://www.mongodb.com/docs/"));
        assert!(find("$and", Kind::QUERY).is_some());
        assert!(find("$and", Kind::EXPRESSION).is_some(), "also an expression");
        assert!(find("$group", Kind::FIELD_QUERY).is_none());
        let sum = find("$sum", Kind::ACCUMULATOR).unwrap();
        assert!(sum.kind.has(Kind::WINDOW));
    }

    #[test]
    fn keeps_arguments_and_return_types() {
        let unwind = find("$unwind", Kind::STAGE).unwrap();
        assert_eq!(unwind.encode, Encode::Object);
        let names: Vec<&str> = unwind.args.iter().map(|a| a.name).collect();
        assert_eq!(names, ["path", "includeArrayIndex", "preserveNullAndEmptyArrays"]);
        assert!(unwind.args[1].optional);
        let size = find("$size", Kind::FIELD_QUERY).unwrap();
        assert_eq!(size.args[0].types, ["int"]);
        let to_upper = find("$toUpper", Kind::EXPRESSION).unwrap();
        assert_eq!(to_upper.returns, ["string"]);
    }

    #[test]
    fn names_are_unique_within_a_kind() {
        for kind in [
            Kind::FIELD_QUERY,
            Kind::QUERY,
            Kind::STAGE,
            Kind::EXPRESSION,
            Kind::UPDATE,
        ] {
            let mut names: Vec<&str> = of(kind).map(|op| op.name).collect();
            let all = names.len();
            names.dedup();
            assert_eq!(names.len(), all, "duplicate in {kind:?}");
        }
    }

    #[test]
    fn summaries_are_one_short_line() {
        for op in OPS {
            assert!(!op.summary.is_empty() && !op.summary.contains('\n'), "{}", op.name);
            assert!(op.summary.chars().count() <= 110, "{}", op.name);
        }
    }
}
