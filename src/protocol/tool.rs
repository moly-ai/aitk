#[cfg(feature = "json")]
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "json", derive(Serialize, Deserialize))]
pub struct Tool {
    pub name: String,
    pub description: Option<String>,
    /// JSON Schema object defining the expected parameters for the tool.
    /// Stored as a raw JSON string to be agnostic of the serialization library.
    pub input_schema: String,
}

impl Tool {
    pub fn new(name: String, description: Option<String>, input_schema: String) -> Self {
        Tool {
            name,
            description,
            input_schema,
        }
    }

    #[cfg(feature = "json")]
    pub fn input_schema_value(&self) -> serde_json::Result<serde_json::Value> {
        serde_json::from_str(&self.input_schema)
    }
}

// Conversion traits for rmcp interop on native platforms
#[cfg(all(not(target_arch = "wasm32"), feature = "json"))]
impl From<rmcp::model::Tool> for Tool {
    fn from(rmcp_tool: rmcp::model::Tool) -> Self {
        let input_schema =
            serde_json::to_string(&rmcp_tool.input_schema).unwrap_or_else(|_| "{}".to_string());
        Tool {
            name: rmcp_tool.name.into_owned(),
            description: rmcp_tool.description.map(|d| d.into_owned()),
            input_schema,
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "json"))]
impl From<Tool> for rmcp::model::Tool {
    fn from(tool: Tool) -> Self {
        use serde_json::Map;
        use std::sync::Arc;

        let input_schema =
            serde_json::from_str(&tool.input_schema).unwrap_or_else(|_| Arc::new(Map::new()));

        rmcp::model::Tool {
            name: tool.name.into(),
            description: tool.description.map(|d| d.into()),
            input_schema,
            output_schema: None,
            annotations: None,
        }
    }
}

/// Permission status for tool call execution
#[derive(Clone, PartialEq, Debug, Default)]
#[cfg_attr(feature = "json", derive(Serialize, Deserialize))]
pub enum ToolCallPermissionStatus {
    /// Waiting for user decision
    #[default]
    Pending,
    /// User approved execution
    Approved,
    /// User denied execution
    Denied,
}

/// Represents a function/tool call made by the AI
#[derive(Clone, PartialEq, Debug, Default)]
#[cfg_attr(feature = "json", derive(Serialize, Deserialize))]
pub struct ToolCall {
    /// Unique identifier for this tool call
    pub id: String,
    /// Name of the tool/function to call
    pub name: String,
    /// Arguments passed to the tool (JSON string)
    pub arguments: String,
    /// Permission status for this tool call
    #[cfg_attr(feature = "json", serde(default))]
    pub permission_status: ToolCallPermissionStatus,
}

impl ToolCall {
    #[cfg(feature = "json")]
    pub fn arguments_json(&self) -> serde_json::Result<serde_json::Map<String, serde_json::Value>> {
        match serde_json::from_str(&self.arguments)? {
            serde_json::Value::Object(map) => Ok(map),
            _ => Err(serde::de::Error::custom("Arguments must be a JSON object")),
        }
    }
}

/// Represents the result of a tool call execution
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "json", derive(Serialize, Deserialize))]
pub struct ToolResult {
    /// The tool call ID this result corresponds to
    pub tool_call_id: String,
    /// The result content from the tool execution
    pub content: String,
    /// Whether the tool call was successful
    pub is_error: bool,
}
