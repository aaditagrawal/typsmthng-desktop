//! Release history is bundled from the same files used by GitHub publication.
use std::collections::BTreeMap;

use semver::Version;

include!(concat!(env!("OUT_DIR"), "/release_notes.rs"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseNotes {
    pub version: Version,
    pub markdown: String,
}

pub fn release_history() -> Vec<ReleaseNotes> {
    let mut releases: BTreeMap<Version, Vec<&str>> = BTreeMap::new();
    for &(version, markdown) in RAW_RELEASE_NOTES {
        if let Ok(version) = Version::parse(version) {
            let body = markdown.trim();
            let body = body.strip_prefix("# v").map_or(body, |_| {
                body.split_once('\n').map_or("", |(_, body)| body.trim())
            });
            if !body.is_empty() {
                releases.entry(version).or_default().push(body);
            }
        }
    }
    releases
        .into_iter()
        .rev()
        .map(|(version, parts)| ReleaseNotes {
            version,
            markdown: parts.join("\n\n"),
        })
        .collect()
}

/// First launch shows the installed version. Skipped upgrades show every missed release.
pub fn unseen_releases(previous: Option<&str>, current: &str) -> Vec<ReleaseNotes> {
    let Ok(current) = Version::parse(current) else {
        return Vec::new();
    };
    let previous = previous.and_then(|previous| Version::parse(previous).ok());
    release_history()
        .into_iter()
        .filter(|release| {
            release.version <= current
                && previous
                    .as_ref()
                    .map_or(release.version == current, |previous| {
                        &release.version > previous
                    })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_launch_skipped_versions_and_downgrades() {
        let current = env!("CARGO_PKG_VERSION");
        let first = unseen_releases(None, current);
        assert_eq!(first.len(), 1);
        assert!(!first[0].markdown.is_empty());
        assert!(unseen_releases(Some(current), current).is_empty());
        assert!(unseen_releases(Some("99.0.0"), current).is_empty());
        let skipped = unseen_releases(Some("0.1.8"), current);
        assert!(skipped.len() >= 3);
        assert!(skipped
            .windows(2)
            .all(|pair| pair[0].version > pair[1].version));
        assert!(skipped
            .iter()
            .all(|notes| notes.version > Version::new(0, 1, 8)));
        assert_eq!(unseen_releases(Some("bad state"), current), first);
    }
}
