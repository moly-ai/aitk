use serde_json::{Map, Value};

/// Create a formatted summary of tool output for display
pub fn create_tool_output_summary(_tool_name: &str, content: &str) -> String {
    // Try to parse as JSON first for better formatting
    if let Ok(json_value) = serde_json::from_str::<Value>(content) {
        // If it's an object with specific fields, format them nicely
        if let Value::Object(obj) = json_value {
            if let Some(Value::String(summary)) = obj.get("summary") {
                return summary.clone();
            }
            // Otherwise return a truncated pretty print
            if let Ok(pretty) = serde_json::to_string_pretty(&obj) {
                if pretty.len() > 100 {
                    return format!("{}...", &pretty[..100]);
                }
                return pretty;
            }
        }
    }

    // For non-JSON or simple text, truncate if too long
    if content.len() > 100 {
        format!("{}...", &content[..100])
    } else {
        content.to_string()
    }
}

/// Parses a namespaced tool name into server_id and tool_name components
/// "filesystem__read_file" -> ("filesystem", "read_file")
/// "mcp-internet-speed__test-speed" -> ("mcp-internet-speed", "test-speed")
pub fn parse_namespaced_tool_name(
    namespaced_name: &str,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = namespaced_name.splitn(2, "__").collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid namespaced tool name: '{}'. Expected format 'server_id__tool_name'",
            namespaced_name
        )
        .into());
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Converts a namespaced tool name to a display-friendly format for UI
/// "filesystem__read_file" -> "filesystem: read_file"
/// "mcp-internet-speed__test-speed" -> "mcp-internet-speed: test-speed"
pub fn display_name_from_namespaced(namespaced_name: &str) -> String {
    if let Ok((server_id, tool_name)) = parse_namespaced_tool_name(namespaced_name) {
        format!("{}: {}", server_id, tool_name)
    } else {
        // Fallback to original name if parsing fails
        namespaced_name.to_string()
    }
}

/// Parse tool arguments from JSON string to Map
pub fn parse_tool_arguments(arguments: &str) -> Result<Map<String, Value>, String> {
    match serde_json::from_str::<Value>(arguments) {
        Ok(Value::Object(args)) => Ok(args),
        Ok(_) => Err("Arguments must be a JSON object".to_string()),
        Err(e) => Err(format!("Failed to parse arguments: {}", e)),
    }
}
