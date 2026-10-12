//! ↑/↓ through earlier commands, keeping what was being typed.

#[derive(Default)]
pub struct Recall {
    entries: Vec<String>,
    /// Index of the entry shown, while walking back.
    pos: Option<usize>,
    /// What was in the input before walking back.
    draft: String,
}

impl Recall {
    /// Remembers a run command; blanks and immediate repeats are skipped.
    pub fn push(&mut self, command: &str) {
        self.pos = None;
        let command = command.trim();
        if command.is_empty() || self.entries.last().is_some_and(|last| last == command) {
            return;
        }
        self.entries.push(command.to_string());
    }

    /// The previous command, or None at the oldest. `current` is kept as
    /// the draft when the walk starts.
    pub fn prev(&mut self, current: &str) -> Option<String> {
        let pos = match self.pos {
            None if self.entries.is_empty() => return None,
            None => {
                self.draft = current.to_string();
                self.entries.len() - 1
            }
            Some(0) => return None,
            Some(p) => p - 1,
        };
        self.pos = Some(pos);
        Some(self.entries[pos].clone())
    }

    /// The next command, then the draft, then None.
    pub fn next(&mut self) -> Option<String> {
        let pos = self.pos?;
        if pos + 1 < self.entries.len() {
            self.pos = Some(pos + 1);
            Some(self.entries[pos + 1].clone())
        } else {
            self.pos = None;
            Some(std::mem::take(&mut self.draft))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Recall;

    #[test]
    fn walks_back_and_forth_keeping_the_draft() {
        let mut r = Recall::default();
        r.push("a");
        r.push("b");
        assert_eq!(r.prev("half typed"), Some("b".to_string()));
        assert_eq!(r.prev("b"), Some("a".to_string()));
        assert_eq!(r.prev("a"), None, "already at the oldest");
        assert_eq!(r.next(), Some("b".to_string()));
        assert_eq!(r.next(), Some("half typed".to_string()), "back to the draft");
        assert_eq!(r.next(), None);
    }

    #[test]
    fn skips_blanks_and_repeats() {
        let mut r = Recall::default();
        r.push("x");
        r.push("x");
        r.push("   ");
        r.push("y");
        assert_eq!(r.prev(""), Some("y".to_string()));
        assert_eq!(r.prev(""), Some("x".to_string()));
        assert_eq!(r.prev(""), None);
    }

    #[test]
    fn pushing_resets_the_position() {
        let mut r = Recall::default();
        r.push("a");
        r.prev("");
        r.push("b");
        assert_eq!(r.prev(""), Some("b".to_string()));
    }
}
