/// Delimiter used when prefixing aggregated tool names: `{server}__{tool}`.
pub const TOOL_DELIMITER: &str = "__";

/// Prefix a tool name with its source server. Server names must not contain `__`.
pub fn prefix_tool_name(server_name: &str, tool_name: &str) -> String {
    unimplemented!("prefix_tool_name")
}

/// Split `server__tool` into `(server, tool)`. Returns `None` if the delimiter is missing.
pub fn strip_server_prefix(prefixed: &str) -> Option<(&str, &str)> {
    unimplemented!("strip_server_prefix")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_with_double_underscore() {
        assert_eq!(
            prefix_tool_name("github", "list_issues"),
            "github__list_issues"
        );
    }

    #[test]
    fn strips_first_delimiter_only() {
        let (server, tool) = strip_server_prefix("github__list_issues").unwrap();
        assert_eq!(server, "github");
        assert_eq!(tool, "list_issues");
    }

    #[test]
    fn strips_tool_names_that_contain_delimiter() {
        let (server, tool) = strip_server_prefix("krisp__search__meetings").unwrap();
        assert_eq!(server, "krisp");
        assert_eq!(tool, "search__meetings");
    }

    #[test]
    fn strip_returns_none_without_delimiter() {
        assert_eq!(strip_server_prefix("list_issues"), None);
    }
}
