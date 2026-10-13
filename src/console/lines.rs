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

/// One command and what it printed.
#[derive(Debug, Clone)]
pub struct Entry {
    pub db: String,
    pub code: String,
    pub lines: Vec<Line>,
}

/// The whole session as plain text for the results pane: prompts and
/// notes are `//` comments, so it stays valid shell code when copied.
pub fn transcript(entries: &[Entry]) -> String {
    let mut blocks = Vec::new();
    for entry in entries {
        let prompt = format!("{}> ", entry.db);
        let mut out: Vec<String> = entry
            .code
            .lines()
            .enumerate()
            .map(|(i, line)| {
                if i == 0 {
                    format!("// {prompt}{line}")
                } else {
                    format!("// {}{line}", " ".repeat(prompt.len()))
                }
            })
            .collect();
        if !entry.lines.is_empty() {
            out.push(body(&entry.lines));
        }
        blocks.push(out.join("\n"));
    }
    blocks.join("\n\n")
}

/// What a result block shows: values and prints as they are, notes and
/// errors as `//` comments, so the text stays valid shell code.
pub fn body(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|l| match l.kind {
            LineKind::Value | LineKind::Print => l.text.clone(),
            LineKind::Info => format!("// {}", l.text),
            LineKind::Error => format!("// ✖ {}", l.text),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What "copy result" takes: the values and prints, without the notes.
pub fn result_text(lines: &[Line]) -> String {
    lines
        .iter()
        .filter(|l| matches!(l.kind, LineKind::Value | LineKind::Print))
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn failed(lines: &[Line]) -> bool {
    lines.iter().any(|l| l.kind == LineKind::Error)
}

/// A query in one line for a block's header: its first line, with "…" when there is more.
pub fn summary(code: &str) -> String {
    let mut lines = code.lines().map(str::trim).filter(|l| !l.is_empty());
    match (lines.next(), lines.next()) {
        (Some(first), Some(_)) => format!("{first} …"),
        (Some(first), None) => first.to_string(),
        _ => String::new(),
    }
}

pub fn lines(outputs: &[Output]) -> Vec<Line> {
    let mut out = Vec::new();
    let mut push = |text: &str, kind| {
        out.extend(text.split('\n').map(|t| Line {
            text: t.to_string(),
            kind,
        }))
    };
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
    use super::{Entry, Line, LineKind, body, failed, lines, result_text, summary, transcript};
    use crate::console::engine::Output;
    use serde_json::json;

    fn l(text: &str, kind: LineKind) -> Line {
        Line {
            text: text.into(),
            kind,
        }
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
    fn transcript_is_copyable_text_with_commented_prompts_and_notes() {
        let entries = vec![
            Entry {
                db: "shop".into(),
                code: "db.a.find()\n  .limit(1)".into(),
                lines: vec![l("[]", LineKind::Value), l("Type \"it\" for more", LineKind::Info)],
            },
            Entry {
                db: "shop".into(),
                code: "print('hi'); oops".into(),
                lines: vec![
                    l("hi", LineKind::Print),
                    l("ReferenceError: oops is not defined", LineKind::Error),
                ],
            },
        ];
        assert_eq!(
            transcript(&entries),
            "// shop> db.a.find()\n//         .limit(1)\n[]\n// Type \"it\" for more\n\n// shop> print('hi'); oops\nhi\n// ✖ ReferenceError: oops is not defined"
        );
        assert_eq!(transcript(&[]), "");
    }

    #[test]
    fn scalars_are_one_line() {
        assert_eq!(
            lines(&[Output::Value(json!("text"))]),
            vec![l("\"text\"", LineKind::Value)]
        );
        assert_eq!(lines(&[Output::Value(json!([]))]), vec![l("[]", LineKind::Value)]);
    }

    #[test]
    fn a_block_shows_everything_and_copies_only_the_result() {
        let out = vec![
            l("[", LineKind::Value),
            l("]", LineKind::Value),
            l("hi", LineKind::Print),
            l("Type \"it\" for more", LineKind::Info),
        ];
        assert_eq!(body(&out), "[\n]\nhi\n// Type \"it\" for more");
        assert_eq!(result_text(&out), "[\n]\nhi", "notes aren't part of the result");
        assert!(!failed(&out));
        assert!(failed(&[l("boom", LineKind::Error)]));
        assert_eq!(body(&[l("boom", LineKind::Error)]), "// ✖ boom");
    }

    #[test]
    fn a_query_is_summed_up_in_its_first_line() {
        assert_eq!(summary("db.a.find()"), "db.a.find()");
        assert_eq!(summary("\n  db.a.find()\n  .limit(1)\n"), "db.a.find() …");
        assert_eq!(summary(""), "");
    }
}
