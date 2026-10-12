//! The app's version and its release notes, kept in CHANGELOG.md.

/// The version in Cargo.toml, the one source of truth.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = "https://github.com/yldrmzffr/humon";

/// GitHub's API for the newest published release.
const LATEST_URL: &str = "https://api.github.com/repos/yldrmzffr/humon/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// The release page, opened when the update notice is clicked.
    pub url: String,
}

/// `X.Y.Z` (optionally `vX.Y.Z`) as numbers; anything else, pre-releases
/// included, is None.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim();
    let parts: Vec<&str> = text.strip_prefix('v').unwrap_or(text).split('.').collect();
    match parts[..] {
        [a, b, c] => Some((a.parse().ok()?, b.parse().ok()?, c.parse().ok()?)),
        _ => None,
    }
}

/// Whether `latest` is a newer release than `current`.
pub fn is_newer(current: &str, latest: &str) -> bool {
    matches!((parse_version(current), parse_version(latest)), (Some(c), Some(l)) if l > c)
}

/// The release in a `/releases/latest` reply.
pub fn parse_latest(json: &str) -> Option<Release> {
    let reply: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = reply["tag_name"].as_str()?;
    parse_version(tag)?;
    Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        url: reply["html_url"].as_str()?.to_string(),
    })
}

/// Asks GitHub for the latest release. Blocking; None on any failure, so
/// being offline just means no notice. Uses curl, which macOS, Linux and
/// Windows 10+ ship, rather than adding an HTTP client to the app.
pub fn fetch_latest() -> Option<Release> {
    let out = std::process::Command::new("curl")
        .args(["-sSfL", "--max-time", "5", "-H", "Accept: application/vnd.github+json"])
        .args(["-H", &format!("User-Agent: humon/{VERSION}"), LATEST_URL])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_latest(std::str::from_utf8(&out.stdout).ok()?)
}

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
    use super::{Release, is_newer, parse_latest, release_notes};

    #[test]
    fn newer_compares_numbers_not_text() {
        assert!(is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.9.1", "0.10.0"), "10 > 9, though '1' < '9' as text");
        assert!(is_newer("0.1.0", "v0.1.1"), "a leading v is fine");
        assert!(is_newer("1.2.3", "2.0.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.1.9"));
        // Anything unparsable is never offered as an update.
        assert!(!is_newer("0.1.0", "nightly"));
        assert!(!is_newer("0.1.0", "0.2"));
        assert!(!is_newer("0.1.0", "0.2.0-beta.1"), "pre-releases are not offered");
    }

    #[test]
    #[ignore = "needs internet"]
    fn live_fetch_latest_from_github() {
        let release = super::fetch_latest().expect("GitHub answered with a release");
        assert!(super::parse_version(&release.version).is_some(), "{release:?}");
        assert!(
            release
                .url
                .starts_with("https://github.com/yldrmzffr/humon/releases/tag/v"),
            "{release:?}"
        );
    }

    #[test]
    fn reads_githubs_latest_release() {
        let json = r#"{"tag_name": "v0.2.0", "html_url": "https://github.com/yldrmzffr/humon/releases/tag/v0.2.0", "draft": false, "assets": []}"#;
        assert_eq!(
            parse_latest(json),
            Some(Release {
                version: "0.2.0".into(),
                url: "https://github.com/yldrmzffr/humon/releases/tag/v0.2.0".into()
            })
        );
        assert_eq!(parse_latest(r#"{"message": "Not Found"}"#), None);
        assert_eq!(parse_latest("not json"), None);
    }

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
