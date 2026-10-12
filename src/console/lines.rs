//! Turns engine outputs into the lines the console draws.

use crate::console::engine::Output;
use crate::shell::to_shell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Shell-syntax value text; drawn with syntax colors.
    Value,
    Print,
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub kind: LineKind,
}

pub fn lines(outputs: &[Output]) -> Vec<Line> {
    let mut out = Vec::new();
    let mut push = |text: &str, kind| out.extend(text.split('\n').map(|t| Line { text: t.to_string(), kind }));
    for output in outputs {
        match output {
            Output::Value(v) => push(&to_shell(v, ""), LineKind::Value),
            Output::Print(t) => push(t, LineKind::Print),
            Output::Info(t) => push(t, LineKind::Info),
            Output::Error(t) => push(t, LineKind::Error),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Line, LineKind, lines};
    use crate::console::engine::Output;
    use serde_json::json;

    fn l(text: &str, kind: LineKind) -> Line {
        Line { text: text.into(), kind }
    }

    #[test]
    fn values_print_info_and_errors() {
        let out = vec![
            Output::Print("first\nsecond".into()),
            Output::Value(json!({ "_id": { "$oid": "65a1b2c3d4e5f60718293a4b" }, "n": { "$numberInt": "1" } })),
            Output::Info("Type \"it\" for more".into()),
            Output::Error("boom".into()),
        ];
        assert_eq!(
            lines(&out),
            vec![
                l("first", LineKind::Print),
                l("second", LineKind::Print),
                l("{", LineKind::Value),
                l("  _id: ObjectId(\"65a1b2c3d4e5f60718293a4b\"),", LineKind::Value),
                l("  n: 1", LineKind::Value),
                l("}", LineKind::Value),
                l("Type \"it\" for more", LineKind::Info),
                l("boom", LineKind::Error),
            ]
        );
    }

    #[test]
    fn scalars_are_one_line() {
        assert_eq!(lines(&[Output::Value(json!("text"))]), vec![l("\"text\"", LineKind::Value)]);
        assert_eq!(lines(&[Output::Value(json!([]))]), vec![l("[]", LineKind::Value)]);
    }
}
