use std::collections::HashMap;

/// Per-server tool visibility. Missing keys are public. `false` hides and blocks the tool.
pub fn is_tool_public(permissions: &HashMap<String, bool>, tool_name: &str) -> bool {
    unimplemented!("is_tool_public")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_permission_is_public() {
        let perms = HashMap::new();
        assert!(is_tool_public(&perms, "search"));
    }

    #[test]
    fn explicit_true_is_public() {
        let mut perms = HashMap::new();
        perms.insert("search".into(), true);
        assert!(is_tool_public(&perms, "search"));
    }

    #[test]
    fn explicit_false_is_private() {
        let mut perms = HashMap::new();
        perms.insert("search".into(), false);
        assert!(!is_tool_public(&perms, "search"));
    }
}
