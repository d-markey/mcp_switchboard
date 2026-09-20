//! Tool filtering logic to dynamically restrict tool visibility.
//! This allows administrators to hide dangerous, experimental, or unwanted tools
//! exposed by upstream backends using glob patterns.

use crate::config::ToolsFilter;
use glob::Pattern;

/// Evaluates if a given tool name is allowed under the current configuration filter rules.
///
/// Blacklists take complete precedence: if a name matches any pattern in the blacklist,
/// it is immediately blocked, even if it is also listed in the whitelist. This ensure a safe-by-default behavior
/// when rejecting specific operations.
pub fn is_tool_allowed(tool_name: &str, filter: Option<&ToolsFilter>) -> bool {
    let Some(f) = filter else {
        return true;
    };

    // Check blacklist first (blacklist always wins)
    for pattern_str in &f.blacklist {
        if let Ok(pat) = Pattern::new(pattern_str) {
            if pat.matches(tool_name) {
                return false;
            }
        }
    }

    // Check whitelist if not empty. If a whitelist is specified, a tool name must match
    // at least one whitelist pattern to be accepted.
    if !f.whitelist.is_empty() {
        let mut allowed = false;
        for pattern_str in &f.whitelist {
            if let Ok(pat) = Pattern::new(pattern_str) {
                if pat.matches(tool_name) {
                    allowed = true;
                    break;
                }
            }
        }
        return allowed;
    }

    true
}

/// Iterates over a collection of tool values and filters out any disallowed entries.
pub fn filter_tools(tools: Vec<serde_json::Value>, filter: Option<&ToolsFilter>) -> Vec<serde_json::Value> {
    tools
        .into_iter()
        .filter(|tool| {
            if let Some(name) = tool.get("name").and_then(|v| v.as_str()) {
                is_tool_allowed(name, filter)
            } else {
                // We keep malformed entries that lack a 'name' field to remain resilient.
                // Downstream handlers or validation steps will manage or report these accordingly,
                // matching the established compatibility behavior with existing clients.
                true
            }
        })
        .collect()
}
