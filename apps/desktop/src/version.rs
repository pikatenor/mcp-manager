//! Sidebar footer: package version, plus git revision on development builds.

/// Label shown in the sidebar footer.
///
/// A release is `v{version}`. A development build appends the git revision.
pub(crate) fn version_label(package_version: &str, git_revision: Option<&str>) -> String {
    match git_revision {
        Some(rev) => format!("v{package_version} ({rev})"),
        None => format!("v{package_version}"),
    }
}

/// Git revision to show when this is not a tagged release of `package_version`.
///
/// `head_tag` is the exact tag on HEAD, if any (`v0.2.1` or `0.2.1`).
/// An empty `git_revision` is treated as missing.
pub(crate) fn development_revision<'a>(
    package_version: &str,
    head_tag: Option<&str>,
    git_revision: &'a str,
) -> Option<&'a str> {
    if git_revision.is_empty() || is_release_tag(package_version, head_tag) {
        None
    } else {
        Some(git_revision)
    }
}

fn is_release_tag(package_version: &str, head_tag: Option<&str>) -> bool {
    let Some(tag) = head_tag else {
        return false;
    };
    tag == package_version || tag.strip_prefix('v') == Some(package_version)
}

/// Sidebar footer for this binary: package version, plus git revision when untagged.
pub(crate) fn sidebar_version() -> String {
    version_label(
        env!("CARGO_PKG_VERSION"),
        development_revision(
            env!("CARGO_PKG_VERSION"),
            option_env!("MCP_MANAGER_GIT_HEAD_TAG"),
            option_env!("MCP_MANAGER_GIT_REVISION").unwrap_or(""),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_label_is_v_prefixed_package_version_for_a_release() {
        assert_eq!(version_label("0.2.1", None), "v0.2.1");
    }

    #[test]
    fn version_label_appends_git_revision_for_a_development_build() {
        assert_eq!(version_label("0.2.1", Some("abc1234")), "v0.2.1 (abc1234)");
    }

    #[test]
    fn development_revision_is_omitted_when_head_is_the_v_prefixed_release_tag() {
        assert_eq!(
            development_revision("0.2.1", Some("v0.2.1"), "abc1234"),
            None
        );
    }

    #[test]
    fn development_revision_is_omitted_when_head_is_the_unprefixed_release_tag() {
        assert_eq!(
            development_revision("0.2.1", Some("0.2.1"), "abc1234"),
            None
        );
    }

    #[test]
    fn development_revision_is_shown_when_head_is_untagged() {
        assert_eq!(
            development_revision("0.2.1", None, "abc1234"),
            Some("abc1234")
        );
    }

    #[test]
    fn development_revision_is_shown_when_head_tag_is_a_different_version() {
        assert_eq!(
            development_revision("0.2.1", Some("v0.2.0"), "abc1234"),
            Some("abc1234")
        );
    }

    #[test]
    fn development_revision_is_omitted_when_git_revision_is_empty() {
        assert_eq!(development_revision("0.2.1", None, ""), None);
    }
}
