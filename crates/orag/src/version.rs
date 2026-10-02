//! Build and API version information.

use serde::Serialize;

/// Crate version; inherited from `[workspace.package] version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Short git commit of the build (12+ hex digits), or `unknown` when it is not
/// built from ORAG's own git checkout, before the first commit, or when git
/// failed (build.rs then prints a cargo warning).
pub const GIT_SHA: &str = env!("ORAG_GIT_SHA");
/// HTTP contract version; changes only on breaking API changes.
pub const API_VERSION: &str = "v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionInfo {
    pub version: &'static str,
    pub api: &'static str,
    pub schema_version: Option<u32>,
    pub git_sha: &'static str,
}

pub fn version_info(schema_version: Option<u32>) -> VersionInfo {
    VersionInfo {
        version: VERSION,
        api: API_VERSION,
        schema_version,
        git_sha: GIT_SHA,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changelog_top_entry_matches_crate_version() {
        let changelog = include_str!("../../../CHANGELOG.md");
        let top = changelog
            .lines()
            .filter_map(|line| line.strip_prefix("## ["))
            .filter_map(|rest| rest.split_once(']').map(|(heading, _)| heading))
            .next()
            .expect("CHANGELOG.md has a version heading");
        assert_eq!(top, VERSION, "bump with scripts/bump-version.sh");
    }
}
