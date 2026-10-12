//! The app's version and its release notes, kept in CHANGELOG.md.

/// The version in Cargo.toml, the one source of truth.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The notes under `## [version]` in a Keep a Changelog file, trimmed.
pub fn release_notes(changelog: &str, version: &str) -> Option<String> {
    let heading = format!("## [{version}]");
    let mut lines = changelog.lines().skip_while(|l| !l.starts_with(&heading));
    lines.next()?;
    let body: Vec<&str> = lines.take_while(|l| !l.starts_with("## [")).collect();
    Some(body.join("\n").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::release_notes;

    #[test]
    fn the_current_version_has_changelog_notes() {
        // Releases take their notes from CHANGELOG.md; none means no release.
        let changelog = include_str!("../CHANGELOG.md");
        let heading = format!("## [{}]", env!("CARGO_PKG_VERSION"));
        assert!(
            changelog.lines().any(|l| l.starts_with(&heading)),
            "CHANGELOG.md has no `{heading}` section"
        );
        assert_eq!(
            release_notes(changelog, env!("CARGO_PKG_VERSION")).map(|n| n.is_empty()),
            Some(false)
        );
    }

    #[test]
    fn release_notes_are_the_versions_section() {
        let changelog =
            "# Changelog\n\n## [0.2.0] - 2026-11-01\n\n- New thing\n- Fix\n\n## [0.1.0] - 2026-10-12\n\n- First\n";
        assert_eq!(release_notes(changelog, "0.2.0").as_deref(), Some("- New thing\n- Fix"));
        assert_eq!(release_notes(changelog, "0.1.0").as_deref(), Some("- First"));
        assert_eq!(release_notes(changelog, "9.9.9"), None);
    }
}
