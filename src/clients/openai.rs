use crate::protocol::Tool;
use async_stream::stream;
use reqwest::header::{HeaderMap, HeaderName};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, RwLock},
};

use crate::protocol::*;
use crate::utils::asynchronous::{BoxPlatformSendFuture, BoxPlatformSendStream};
use crate::utils::{serde::deserialize_null_default, sse::parse_sse};

/// The content of a [`ContentPart::ImageUrl`].
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ImageUrlDetail {
    url: String,
    // #[serde(skip_serializing_if = "Option::is_none")]
    // detail: Option<String>,
}

/// The content of a [`ContentPart::File`].
#[derive(Serialize, Deserialize, Debug, Clone)]
struct File {
    filename: String,
    file_data: String,
}

/// Represents a single part in a multi-part content array of [`Content`].
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrlDetail },
    File { file: File },
}

/// Represents the 'content' field, which can be a string or an array of ContentPart
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)] // Tells Serde to try deserializing into variants without a specific tag
enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl Default for Content {
    fn default() -> Self {
        Content::Text(String::new())
    }
}

impl Content {
    /// Returns the text content if available, otherwise an empty string.
    pub fn text(&self) -> String {
        match self {
            Content::Text(text) => text.clone(),
            Content::Parts(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<String>>()
                .join(" "),
        }
    }
}

#[derive(Serialize)]
struct FunctionDefinition {
    name: String,
    description: String,
    parameters: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    strict: Option<bool>,
}

/// Tool definition for OpenAI API
#[derive(Serialize)]
struct FunctionTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: FunctionDefinition,
}

impl From<&Tool> for FunctionTool {
    fn from(tool: &Tool) -> Self {
        // Use the input_schema from the MCP tool, but ensure OpenAI compatibility
        let mut parameters_map = (*tool.input_schema).clone();

        // Ensure additionalProperties is set to false as required by OpenAI
        parameters_map.insert(
            "additionalProperties".to_string(),
            serde_json::Value::Bool(false),
        );

        // Ensure properties field exists for object schemas (OpenAI requirement)
        if parameters_map.get("type") == Some(&serde_json::Value::String("object".to_string())) {
            if !parameters_map.contains_key("properties") {
                parameters_map.insert(
                    "properties".to_string(),
                    serde_json::Value::Object(serde_json::Map::new()),
                );
            }
        }

        let parameters = serde_json::Value::Object(parameters_map);

        FunctionTool {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: tool.name.clone(),
                description: tool.description.as_deref().unwrap_or("").to_string(),
                parameters,
                strict: Some(false),
            },
        }
    }
}

/// Tool call from OpenAI API
#[derive(Clone, Debug, Deserialize)]
struct OpenAiToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type")]
    #[serde(default)]
    #[allow(dead_code)] // tool_type is necessary for the OpenAI, but we don't use it
    pub tool_type: String,
    pub function: OpenAiFunctionCall,
}

/// Function call within a tool call
#[derive(Clone, Debug, Deserialize)]
struct OpenAiFunctionCall {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub arguments: String, // JSON string that needs to be parsed
}

/// Message being received by the completions endpoint.
///
/// Although most OpenAI-compatible APIs return a `role` field, OpenAI itself does not.
///
/// Also, OpenAI may return an empty object as `delta` while streaming, that's why
/// content is optional.
///
/// And SiliconFlow may set `content` to a `null` value, that's why the custom deserializer
/// is needed.
#[derive(Clone, Debug, Deserialize)]
struct IncomingMessage {
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_null_default")]
    pub content: Content,
    /// The reasoning text from providers that use `reasoning` (e.g. OpenRouter).
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_null_default")]
    pub reasoning: String,
    /// The reasoning text from providers that use `reasoning_content`
    /// (e.g. SiliconFlow, NVIDIA NIM). Some providers send both fields
    /// with identical content, so these must be separate to avoid
    /// serde's "duplicate field" error.
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_null_default")]
    pub reasoning_content: String,
    /// Tool calls made by the assistant.
    /// Some providers (e.g. NVIDIA NIM) may set this to `null` instead of
    /// omitting it, so a null-safe deserializer is needed.
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_null_default")]
    pub tool_calls: Vec<OpenAiToolCall>,
}
/// A message being sent to the completions endpoint.
#[derive(Clone, Debug, Serialize)]
struct OutgoingMessage {
    pub content: Content,
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

async fn to_outgoing_message(message: Message) -> Result<OutgoingMessage, String> {
    // Handle tool results differently
    if !message.content.tool_results.is_empty() {
        return outgoing_tool_result_message(message);
    }

    let role = match message.from {
        EntityId::User => Ok(Role::User),
        EntityId::System => Ok(Role::System),
        EntityId::Bot(_) => Ok(Role::Assistant),
        EntityId::Tool => Ok(Role::Tool),
        EntityId::App => Err("App messages cannot be sent to OpenAI".to_string()),
    }?;

    let content = if message.content.attachments.is_empty() {
        Content::Text(message.content.text)
    } else {
        let mut parts = Vec::new();

        for attachment in message.content.attachments {
            if !attachment.is_available() {
                log::warn!("Skipping unavailable attachment: {}", attachment.name);
                continue;
            }

            let content = attachment
                .read_base64()
                .await
                .map_err(|e| format!("Failed to read attachment '{}': {}", attachment.name, e))?;
            let data_url = format!(
                "data:{};base64,{}",
                attachment
                    .content_type
                    .as_deref()
                    .unwrap_or("application/octet-stream"),
                content
            );

            if attachment.is_image() {
                parts.push(ContentPart::ImageUrl {
                    image_url: ImageUrlDetail { url: data_url },
                });
            } else if attachment.is_pdf() {
                parts.push(ContentPart::File {
                    file: File {
                        filename: attachment.name,
                        file_data: data_url,
                    },
                });
            } else {
                // For text-based files (HTML, MD, TXT, etc), decode and include as text
                match decode_base64_to_text(&content) {
                    Ok(text_content) => {
                        parts.push(ContentPart::Text {
                            text: format!("[File: {}]\n{}", attachment.name, text_content),
                        });
                    }
                    Err(_) => {
                        // This file is not text-decodable (likely binary), return error
                        return Err(format!(
                            "File '{}' is not supported. Only images, PDFs, and text files can be sent through the Chat Completions API.",
                            attachment.name
                        ));
                    }
                }
            }
        }

        parts.push(ContentPart::Text {
            text: message.content.text,
        });
        Content::Parts(parts)
    };

    // Convert tool calls to OpenAI format
    let tool_calls =
        if !message.content.tool_calls.is_empty() {
            Some(message.content.tool_calls.iter().map(|tc| {
            serde_json::json!({
                "id": tc.id,
                "type": "function",
                "function": {
                    "name": tc.name,
                    "arguments": serde_json::to_string(&tc.arguments).unwrap_or_default()
                }
            })
        }).collect())
        } else {
            None
        };

    Ok(OutgoingMessage {
        content,
        role,
        tool_calls,
        tool_call_id: None,
    })
}

/// Converts a message with tool results to an outgoing message.
///
/// This is used to send tool results back to the AI.
fn outgoing_tool_result_message(message: Message) -> Result<OutgoingMessage, String> {
    let role = Role::Tool;
    let content = Content::Text(
        message
            .content
            .tool_results
            .iter()
            .map(|result| truncate_tool_result(&result.content))
            .collect::<Vec<_>>()
            .join("\n"),
    );

    let tool_call_id = message
        .content
        .tool_results
        .first()
        .map(|r| r.tool_call_id.clone());

    return Ok(OutgoingMessage {
        content,
        role,
        tool_calls: None,
        tool_call_id,
    });
}

fn truncate_tool_result(content: &str) -> String {
    const MAX_TOOL_OUTPUT_CHARS: usize = 16384; // ~4096 tokens
    if content.len() > MAX_TOOL_OUTPUT_CHARS {
        let truncated = content
            .chars()
            .take(MAX_TOOL_OUTPUT_CHARS)
            .collect::<String>();
        format!("{}... [truncated]", truncated)
    } else {
        content.to_string()
    }
}

/// Decode base64-encoded content to UTF-8 text.
/// Returns an error if the content is not valid UTF-8 text.
fn decode_base64_to_text(base64: &str) -> Result<String, ()> {
    use base64::Engine;

    // Decode base64 to bytes
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64)
        .map_err(|_| ())?;

    // Convert bytes to UTF-8 string
    String::from_utf8(bytes).map_err(|_| ())
}

/// Finalizes any remaining buffered tool calls when streaming completes.
/// This processes incomplete tool calls that were being built up during streaming.
fn finalize_remaining_tool_calls(
    content: &mut MessageContent,
    tool_argument_buffers: &mut HashMap<String, String>,
    tool_names: &mut HashMap<String, String>,
    tool_call_ids_by_index: &mut HashMap<usize, String>,
) {
    // Process any remaining buffered tool calls
    for (tool_call_id, buffered_args) in tool_argument_buffers.drain() {
        let arguments = if buffered_args.is_empty() || buffered_args == "{}" {
            serde_json::Map::new()
        } else {
            match serde_json::from_str::<serde_json::Value>(&buffered_args) {
                Ok(serde_json::Value::Object(args)) => args,
                Ok(serde_json::Value::Null) => serde_json::Map::new(),
                Ok(_) => serde_json::Map::new(),
                Err(_) => serde_json::Map::new(),
            }
        };

        // Create the tool call if we have the name and it's not already created
        if let Some(name) = tool_names.get(&tool_call_id) {
            let tool_call = ToolCall {
                id: tool_call_id.clone(),
                name: name.clone(),
                arguments,
                ..Default::default()
            };
            content.tool_calls.push(tool_call);
        }
    }

    // Clear the tool names and index mapping as well
    tool_names.clear();
    tool_call_ids_by_index.clear();
}

/// Role of a message that is part of the conversation context.
#[derive(Clone, Debug, Serialize, Deserialize)]
enum Role {
    /// OpenAI o1 models seems to expect `developer` instead of `system` according
    /// to the documentation. But it also seems like `system` is converted to `developer`
    /// internally.
    #[serde(rename = "system")]
    System,
    #[serde(rename = "user")]
    User,
    #[serde(rename = "assistant")]
    Assistant,
    #[serde(rename = "tool")]
    Tool,
}

/// The Choice object as part of a streaming response.
#[derive(Clone, Debug, Deserialize)]
struct Choice {
    pub delta: IncomingMessage,
    pub finish_reason: Option<String>,
}

/// Response from the completions endpoint
#[derive(Clone, Debug, Deserialize)]
struct Completion {
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub citations: Vec<String>,
}

/// Controls how tools are selected when calling the OpenAI Chat Completions API.
#[derive(Clone, Debug, PartialEq)]
pub enum OpenAiToolChoice {
    /// The model must not call tools.
    None,
    /// Let the model decide whether to call tools.
    Auto,
    /// The model must call one or more tools.
    Required,
    /// Force a specific tool by function name.
    Function { name: String },
}

impl Serialize for OpenAiToolChoice {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match self {
            OpenAiToolChoice::None => {
                serializer.serialize_str("none")
            }
            OpenAiToolChoice::Auto => {
                serializer.serialize_str("auto")
            }
            OpenAiToolChoice::Required => {
                serializer.serialize_str("required")
            }
            OpenAiToolChoice::Function { name } => {
                use serde::ser::SerializeMap;
                let mut map =
                    serializer.serialize_map(Some(2))?;
                map.serialize_entry(
                    "type",
                    "function",
                )?;
                map.serialize_entry(
                    "function",
                    &serde_json::json!({"name": name}),
                )?;
                map.end()
            }
        }
    }
}

/// JSON schema definition used by [`OpenAiResponseFormat::JsonSchema`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpenAiJsonSchemaResponseFormat {
    /// Schema identifier expected by OpenAI-compatible APIs.
    pub name: String,
    /// JSON Schema document.
    pub schema: serde_json::Value,
    /// Whether the model output should strictly follow the schema.
    pub strict: bool,
}

/// Controls structured output behavior for the Chat Completions API.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpenAiResponseFormat {
    /// Default text output.
    Text,
    /// JSON object output mode.
    JsonObject,
    /// Structured output using JSON Schema.
    JsonSchema {
        /// The JSON schema configuration.
        json_schema: OpenAiJsonSchemaResponseFormat,
    },
}

impl OpenAiResponseFormat {
    /// Creates a JSON schema response format payload.
    pub fn json_schema(
        name: String,
        schema: serde_json::Value,
        strict: bool,
    ) -> Self {
        OpenAiResponseFormat::JsonSchema {
            json_schema: OpenAiJsonSchemaResponseFormat {
                name,
                schema,
                strict,
            },
        }
    }
}

/// Configurable OpenAI Chat Completions request options.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenAiRequestOptions {
    /// Optional `tool_choice` field sent to the API.
    pub tool_choice: Option<OpenAiToolChoice>,
    /// Optional `response_format` field sent to the API.
    pub response_format: Option<OpenAiResponseFormat>,
    /// Sampling temperature (`0.0..=2.0`). Higher values produce more random output.
    pub temperature: Option<f32>,
    /// Nucleus sampling threshold (`0.0..=1.0`).
    pub top_p: Option<f32>,
    /// Maximum number of tokens to generate in the completion.
    pub max_completion_tokens: Option<u32>,
    /// Stop sequences (up to 4). Generation stops when any sequence is encountered.
    pub stop: Option<OpenAiStop>,
    /// Optional `parallel_tool_calls` field sent to the API.
    pub parallel_tool_calls: Option<bool>,
    /// Optional `seed` field sent to the API.
    pub seed: Option<i64>,
    /// Penalizes tokens based on prior appearance in the text (`-2.0..=2.0`).
    pub presence_penalty: Option<f32>,
    /// Penalizes tokens based on their frequency in the text (`-2.0..=2.0`).
    pub frequency_penalty: Option<f32>,
    /// Whether to return log probabilities of output tokens.
    pub logprobs: Option<bool>,
    /// Number of most likely tokens to return at each position (`0..=20`).
    /// Requires `logprobs` to be `true`.
    pub top_logprobs: Option<u8>,
    /// End-user identifier for OpenAI abuse monitoring.
    pub user: Option<String>,
    /// Stream-specific options such as `include_usage`.
    pub stream_options: Option<OpenAiStreamOptions>,
}

/// Stop sequences used by the OpenAI Chat Completions API.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum OpenAiStop {
    /// A single stop sequence.
    Single(String),
    /// Multiple stop sequences (up to 4).
    Multiple(Vec<String>),
}

/// Stream-specific options for the Chat Completions API.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct OpenAiStreamOptions {
    /// Whether to include token usage data in stream responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_usage: Option<bool>,
}

impl OpenAiRequestOptions {
    /// Returns options with `tool_choice` configured.
    pub fn with_tool_choice(mut self, tool_choice: OpenAiToolChoice) -> Self {
        self.tool_choice = Some(tool_choice);
        self
    }

    /// Returns options with `response_format` configured.
    pub fn with_response_format(mut self, response_format: OpenAiResponseFormat) -> Self {
        self.response_format = Some(response_format);
        self
    }

    /// Returns options with sampling temperature (`0.0..=2.0`) configured.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Returns options with nucleus sampling threshold (`0.0..=1.0`) configured.
    pub fn with_top_p(mut self, top_p: f32) -> Self {
        self.top_p = Some(top_p);
        self
    }

    /// Returns options with `max_completion_tokens` configured.
    pub fn with_max_completion_tokens(mut self, max_completion_tokens: u32) -> Self {
        self.max_completion_tokens = Some(max_completion_tokens);
        self
    }

    /// Returns options with `stop` configured.
    pub fn with_stop(mut self, stop: OpenAiStop) -> Self {
        self.stop = Some(stop);
        self
    }

    /// Returns options with `parallel_tool_calls` configured.
    pub fn with_parallel_tool_calls(mut self, parallel_tool_calls: bool) -> Self {
        self.parallel_tool_calls = Some(parallel_tool_calls);
        self
    }

    /// Returns options with `seed` configured.
    pub fn with_seed(mut self, seed: i64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Returns options with presence penalty (`-2.0..=2.0`) configured.
    pub fn with_presence_penalty(mut self, presence_penalty: f32) -> Self {
        self.presence_penalty = Some(presence_penalty);
        self
    }

    /// Returns options with frequency penalty (`-2.0..=2.0`) configured.
    pub fn with_frequency_penalty(mut self, frequency_penalty: f32) -> Self {
        self.frequency_penalty = Some(frequency_penalty);
        self
    }

    /// Returns options with `logprobs` configured.
    pub fn with_logprobs(mut self, logprobs: bool) -> Self {
        self.logprobs = Some(logprobs);
        self
    }

    /// Returns options with `top_logprobs` configured (`0..=20`).
    pub fn with_top_logprobs(
        mut self,
        top_logprobs: u8,
    ) -> Self {
        self.top_logprobs = Some(top_logprobs);
        self
    }

    /// Returns options with `user` configured.
    pub fn with_user(mut self, user: String) -> Self {
        self.user = Some(user);
        self
    }

    /// Returns options with `stream_options` configured.
    pub fn with_stream_options(
        mut self,
        stream_options: OpenAiStreamOptions,
    ) -> Self {
        self.stream_options = Some(stream_options);
        self
    }
}

fn build_chat_completions_request_body(
    model: &str,
    messages: &[OutgoingMessage],
    tools: &[FunctionTool],
    options: &OpenAiRequestOptions,
) -> serde_json::Value {
    let mut json = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": true
    });

    if !tools.is_empty() {
        json["tools"] = serde_json::json!(tools);

        if let Some(tool_choice) = &options.tool_choice {
            json["tool_choice"] =
                serde_json::to_value(tool_choice)
                    .expect(
                        "OpenAiToolChoice serialization \
                        cannot fail",
                    );
        }

        if let Some(parallel_tool_calls) = options.parallel_tool_calls {
            json["parallel_tool_calls"] =
                serde_json::json!(parallel_tool_calls);
        }
    }

    if let Some(response_format) = &options.response_format {
        json["response_format"] =
            serde_json::to_value(response_format)
                .expect(
                    "OpenAiResponseFormat serialization \
                    cannot fail",
                );
    }

    if let Some(temperature) = options.temperature {
        json["temperature"] = serde_json::json!(temperature);
    }

    if let Some(top_p) = options.top_p {
        json["top_p"] = serde_json::json!(top_p);
    }

    if let Some(max_completion_tokens) = options.max_completion_tokens {
        json["max_completion_tokens"] = serde_json::json!(max_completion_tokens);
    }

    if let Some(stop) = &options.stop {
        json["stop"] =
            serde_json::to_value(stop)
                .expect(
                    "OpenAiStop serialization cannot fail",
                );
    }

    if let Some(seed) = options.seed {
        json["seed"] = serde_json::json!(seed);
    }

    if let Some(presence_penalty) = options.presence_penalty {
        json["presence_penalty"] = serde_json::json!(presence_penalty);
    }

    if let Some(frequency_penalty) = options.frequency_penalty {
        json["frequency_penalty"] = serde_json::json!(frequency_penalty);
    }

    if let Some(logprobs) = options.logprobs {
        json["logprobs"] = serde_json::json!(logprobs);
    }

    if let Some(top_logprobs) = options.top_logprobs {
        json["top_logprobs"] =
            serde_json::json!(top_logprobs);
    }

    if let Some(user) = &options.user {
        json["user"] = serde_json::json!(user);
    }

    if let Some(stream_options) = &options.stream_options {
        json["stream_options"] =
            serde_json::to_value(stream_options).expect(
                "OpenAiStreamOptions serialization cannot fail",
            );
    }

    json
}

#[derive(Clone, Debug)]
struct OpenAiClientInner {
    url: String,
    headers: HeaderMap,
    client: reqwest::Client,
    tools_enabled: bool,
    request_options: OpenAiRequestOptions,
}

/// A client capable of interacting with Moly Server and other OpenAI-compatible APIs.
#[derive(Debug)]
pub struct OpenAiClient(Arc<RwLock<OpenAiClientInner>>);

impl Clone for OpenAiClient {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl From<OpenAiClientInner> for OpenAiClient {
    fn from(inner: OpenAiClientInner) -> Self {
        Self(Arc::new(RwLock::new(inner)))
    }
}

impl OpenAiClient {
    /// Creates a new client with the given OpenAI-compatible API URL.
    pub fn new(url: String) -> Self {
        let headers = HeaderMap::new();
        let client = crate::utils::http::default_client();

        OpenAiClientInner {
            url,
            headers,
            client,
            tools_enabled: true, // Default to enabled for backward compatibility
            request_options: OpenAiRequestOptions::default(),
        }
        .into()
    }

    pub fn set_header(&mut self, key: &str, value: &str) -> Result<(), &'static str> {
        let header_name = HeaderName::from_str(key).map_err(|_| "Invalid header name")?;

        let header_value = value.parse().map_err(|_| "Invalid header value")?;

        self.0
            .write()
            .expect("openai client lock poisoned")
            .headers
            .insert(header_name, header_value);

        Ok(())
    }

    pub fn set_key(&mut self, key: &str) -> Result<(), &'static str> {
        self.set_header("Authorization", &format!("Bearer {}", key))?;

        // Anthropic requires a different header for the API key, even with the OpenAI API compatibility layer.
        let is_anthropic = self.0
            .read()
            .expect("openai client lock poisoned")
            .url
            .contains("anthropic");
        if is_anthropic {
            self.set_header("x-api-key", key)?;
            // Also needed for every Anthropic request.
            // TODO: remove this once we support a native Anthropic client.
            self.set_header("anthropic-version", "2023-06-01")?;
        }

        Ok(())
    }

    pub fn set_tools_enabled(&mut self, enabled: bool) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .tools_enabled = enabled;
    }

    /// Replaces all request options used by future chat completion requests.
    pub fn set_request_options(&mut self, options: OpenAiRequestOptions) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options = options;
    }

    /// Sets the `tool_choice` option for future chat completion requests.
    pub fn set_tool_choice(&mut self, tool_choice: Option<OpenAiToolChoice>) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .tool_choice = tool_choice;
    }

    /// Sets the `response_format` option for future chat completion requests.
    pub fn set_response_format(&mut self, response_format: Option<OpenAiResponseFormat>) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .response_format = response_format;
    }

    /// Sets the sampling temperature (`0.0..=2.0`) for future requests.
    pub fn set_temperature(&mut self, temperature: Option<f32>) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .temperature = temperature;
    }

    /// Sets the nucleus sampling threshold (`0.0..=1.0`) for future requests.
    pub fn set_top_p(&mut self, top_p: Option<f32>) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .top_p = top_p;
    }

    /// Sets the max completion tokens for future requests.
    pub fn set_max_completion_tokens(&mut self, max_completion_tokens: Option<u32>) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .max_completion_tokens = max_completion_tokens;
    }

    /// Sets stop sequences (up to 4) for future requests.
    pub fn set_stop(&mut self, stop: Option<OpenAiStop>) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .stop = stop;
    }

    /// Sets the `parallel_tool_calls` option for future chat completion requests.
    pub fn set_parallel_tool_calls(&mut self, parallel_tool_calls: Option<bool>) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .parallel_tool_calls = parallel_tool_calls;
    }

    /// Sets the `seed` option for future chat completion requests.
    pub fn set_seed(&mut self, seed: Option<i64>) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .seed = seed;
    }

    /// Sets the presence penalty (`-2.0..=2.0`) for future requests.
    pub fn set_presence_penalty(&mut self, presence_penalty: Option<f32>) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .presence_penalty = presence_penalty;
    }

    /// Sets the frequency penalty (`-2.0..=2.0`) for future requests.
    pub fn set_frequency_penalty(&mut self, frequency_penalty: Option<f32>) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .frequency_penalty = frequency_penalty;
    }

    /// Sets the `logprobs` option for future requests.
    pub fn set_logprobs(&mut self, logprobs: Option<bool>) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .logprobs = logprobs;
    }

    /// Sets the `top_logprobs` option (`0..=20`) for future
    /// requests. Requires `logprobs` to be `Some(true)`.
    pub fn set_top_logprobs(
        &mut self,
        top_logprobs: Option<u8>,
    ) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .top_logprobs = top_logprobs;
    }

    /// Sets the `user` option for future requests.
    pub fn set_user(&mut self, user: Option<String>) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .user = user;
    }

    /// Sets `stream_options` for future requests.
    pub fn set_stream_options(
        &mut self,
        stream_options: Option<OpenAiStreamOptions>,
    ) {
        self.0
            .write()
            .expect("openai client lock poisoned")
            .request_options
            .stream_options = stream_options;
    }
}

impl BotClient for OpenAiClient {
    fn bots(&mut self) -> BoxPlatformSendFuture<'static, ClientResult<Vec<Bot>>> {
        let inner = self.0
            .read()
            .expect("openai client lock poisoned")
            .clone();
        let client = inner.client;
        let base_url = inner.url;
        let headers = inner.headers;

        Box::pin(async move {
            let capabilities = BotCapabilities::new().with_capabilities([
                BotCapability::TextInput,
                BotCapability::AttachmentInput,
                BotCapability::ToolInput,
            ]);

            crate::utils::openai::get_bots(&client, &base_url, headers, &capabilities)
                .await
                .into()
        })
    }

    fn clone_box(&self) -> Box<dyn BotClient> {
        Box::new(self.clone())
    }

    /// Stream pieces of content back as a ChatDelta instead of just a String.
    fn send(
        &mut self,
        bot_id: &BotId,
        messages: &[Message],
        tools: &[Tool],
    ) -> BoxPlatformSendStream<'static, ClientResult<MessageContent>> {
        let bot_id = bot_id.clone();
        let messages = messages.to_vec();

        let inner = self.0
            .read()
            .expect("openai client lock poisoned")
            .clone();
        let url = format!("{}/chat/completions", inner.url);
        let headers = inner.headers;
        let request_options = inner.request_options;

        // Only process tools if they are enabled for this client
        let tools: Vec<FunctionTool> = if inner.tools_enabled {
            tools.iter().map(|t| t.into()).collect()
        } else {
            Vec::new()
        };

        let stream = stream! {
            let mut outgoing_messages: Vec<OutgoingMessage> = Vec::with_capacity(messages.len());
            for message in messages {
                match to_outgoing_message(message.clone()).await {
                    Ok(outgoing_message) => outgoing_messages.push(outgoing_message),
                    Err(err) => {
                        log::error!("Could not convert message to outgoing format: {}", err);
                        yield ClientError::new(
                            ClientErrorKind::Format,
                            err,
                        ).into();
                        return;
                    }
                }
            }

            let json = build_chat_completions_request_body(
                bot_id.id(),
                &outgoing_messages,
                &tools,
                &request_options,
            );

            let request = inner
                .client
                .post(&url)
                .headers(headers)
                .json(&json);

            let response = match request.send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        response
                    } else {
                        let status_code = response.status();
                        let body = response.text().await.unwrap();
                        let message = format!(
                            "Request failed with status {}",
                            status_code,
                        );

                        log::error!("Error sending request to {}: status {}", url, status_code);
                        yield ClientError::new(
                            ClientErrorKind::Response,
                            message,
                        ).with_details(body).into();
                        return;
                    }
                }
                Err(error) => {
                    log::error!("Error sending request to {}: {:?}", url, error);
                    yield ClientError::new_with_source(
                        ClientErrorKind::Network,
                        format!("Could not send request to {url}. Verify your connection and the server status."),
                        Some(error),
                    ).into();
                    return;
                }
            };

            let mut content = MessageContent::default();
            let mut full_text = String::default();
                            let mut tool_argument_buffers: HashMap<String, String> = HashMap::new();
                let mut tool_names: HashMap<String, String> = HashMap::new();
                let mut tool_call_ids_by_index: HashMap<usize, String> = HashMap::new();
            let events = parse_sse(response.bytes_stream());

            for await event in events {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        log::error!("Response streaming got interrupted while reading from {}: {:?}", url, error);
                        yield ClientError::new_with_source(
                            ClientErrorKind::Network,
                            format!("Response streaming got interrupted while reading from {url}. This may be a problem with your connection or the server."),
                            Some(error),
                        ).into();
                        return;
                    }
                };

                let completion: Completion = match serde_json::from_str(&event) {
                    Ok(c) => c,
                    Err(error) => {
                        log::error!("Could not parse the SSE message from {url} as JSON or its structure does not match the expected format. {}", error);
                        yield ClientError::new_with_source(
                            ClientErrorKind::Format,
                            format!("Could not parse the SSE message from {url} as JSON or its structure does not match the expected format."),
                            Some(error),
                        ).into();
                        return;
                    }
                };

                // Check if this chunk has finish_reason for tool_calls
                let is_tool_calls_finished = completion.choices.iter()
                    .any(|choice| choice.finish_reason.as_deref() == Some("tool_calls"));

                let mut should_yield_content = true;

                if is_tool_calls_finished {
                    finalize_remaining_tool_calls(
                        &mut content,
                        &mut tool_argument_buffers,
                        &mut tool_names,
                        &mut tool_call_ids_by_index,
                    );
                } else if !tool_argument_buffers.is_empty() || !tool_names.is_empty() {
                    // We have incomplete tool calls, don't yield content yet
                    should_yield_content = false;
                }

                // Aggregate deltas
                for choice in &completion.choices {
                    // Keep track of the full content as it came, without modifications.
                    full_text.push_str(&choice.delta.content.text());

                    // Extract the inlined reasoning if any.
                    let (reasoning, text) = split_reasoning_tag(&full_text);

                    // Set the content text without any reasoning.
                    content.text = text.to_string();

                    if reasoning.is_empty() {
                        // Append reasoning delta if reasoning was not part of the content.
                        // Some providers use `reasoning`, others use `reasoning_content`,
                        // and some (NVIDIA NIM) send both with identical content.
                        let delta_reasoning = if !choice.delta.reasoning.is_empty() {
                            &choice.delta.reasoning
                        } else {
                            &choice.delta.reasoning_content
                        };
                        content.reasoning.push_str(delta_reasoning);
                    } else {
                        // Otherwise, set the reasoning to what we extracted from the full text.
                        content.reasoning = reasoning.to_string();
                    }

                    // Handle tool calls
                    for (index, tool_call) in choice.delta.tool_calls.iter().enumerate() {
                        // Determine the tool call ID to use
                        let tool_call_id = if !tool_call.id.is_empty() {
                            // This chunk has an ID, use it and store the index mapping
                            tool_call_ids_by_index.insert(index, tool_call.id.clone());
                            tool_call.id.clone()
                        } else {
                            // This chunk doesn't have an ID, look it up by index
                            if let Some(existing_id) = tool_call_ids_by_index.get(&index) {
                                existing_id.clone()
                            } else {
                                continue;
                            }
                        };

                        // Update the argument buffer for this tool call
                        let buffer_entry = tool_argument_buffers.entry(tool_call_id.clone()).or_default();
                        buffer_entry.push_str(&tool_call.function.arguments);

                        // If this chunk has a function name, it's the initial tool call definition
                        // Store the name but don't add to content.tool_calls yet, wait until arguments are complete
                        if !tool_call.function.name.is_empty() {
                            tool_names.insert(tool_call_id.clone(), tool_call.function.name.clone());
                        }

                        // Try to parse the current buffer as complete JSON
                        if !buffer_entry.is_empty() {
                            // Determine the arguments to use based on the buffer content
                            let arguments = if buffer_entry == "{}" {
                                // Special case: Empty JSON object indicates a tool call with no arguments
                                // Example: A tool like "get_weather" that takes no parameters
                                Some(serde_json::Map::new())
                            } else {
                                match serde_json::from_str::<serde_json::Value>(buffer_entry) {
                                    // Successfully parsed as a JSON object with key-value pairs
                                    // This is the normal case for tool calls with parameters
                                    // Example: {"query": "What's the weather?", "location": "NYC"}
                                    Ok(serde_json::Value::Object(args)) => Some(args),
                                    // Successfully parsed as JSON null value
                                    // Treat this the same as empty object - tool call with no arguments
                                    Ok(serde_json::Value::Null) => Some(serde_json::Map::new()),
                                    // Successfully parsed as some other JSON type (array, string, number, bool)
                                    // This is unexpected for tool arguments, so we default to empty arguments for now
                                    Ok(_) => Some(serde_json::Map::new()),
                                    // Failed to parse as valid JSON - arguments are still incomplete
                                    // This happens when we're in the middle of streaming and haven't
                                    // received the complete JSON yet. Keep buffering until we can parse.
                                    Err(_) => None,
                                }
                            };

                            // Create and finalize the tool call if arguments are ready
                            if let (Some(arguments), Some(name)) = (arguments, tool_names.get(&tool_call_id)) {
                                let tool_call = ToolCall {
                                    id: tool_call_id.clone(),
                                    name: name.clone(),
                                    arguments,
                                    ..Default::default()
                                };
                                content.tool_calls.push(tool_call);
                                tool_argument_buffers.remove(&tool_call_id);
                                tool_names.remove(&tool_call_id);
                            }
                        }
                    }
                }

                for citation in completion.citations {
                    if !content.citations.contains(&citation) {
                        content.citations.push(citation.clone());
                    }
                }

                if should_yield_content {
                    yield ClientResult::new_ok(content.clone());
                }
            }
        };

        Box::pin(stream)
    }
}

/// If a string starts with a `<think>` tag, split the content from the rest of the text.
/// - This happens in order, so first element of the tuple is the reasoning.
/// - If the tag is unclosed, everything goes to reasoning.
/// - If there is no tag, everything goes to the second element of the tuple.
fn split_reasoning_tag(text: &str) -> (&str, &str) {
    const START_TAG: &str = "<think>";
    const END_TAG: &str = "</think>";

    if let Some(text) = text.trim_start().strip_prefix(START_TAG) {
        text.split_once(END_TAG).unwrap_or((text, ""))
    } else {
        ("", text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_function_tool() -> FunctionTool {
        FunctionTool {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: "test_fn".to_string(),
                description: "A test function".to_string(),
                parameters: serde_json::json!({"type": "object"}),
                strict: None,
            },
        }
    }

    fn sample_user_message() -> OutgoingMessage {
        OutgoingMessage {
            content: Content::Text("hello".to_string()),
            role: Role::User,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn request_body_omits_optional_fields_by_default() {
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[],
            &OpenAiRequestOptions::default(),
        );

        assert_eq!(body["model"], "gpt-test");
        assert_eq!(body["stream"], true);
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
        assert!(body.get("response_format").is_none());
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        assert!(body.get("max_completion_tokens").is_none());
        assert!(body.get("stop").is_none());
        assert!(body.get("parallel_tool_calls").is_none());
        assert!(body.get("seed").is_none());
        assert!(body.get("presence_penalty").is_none());
        assert!(body.get("frequency_penalty").is_none());
        assert!(body.get("logprobs").is_none());
        assert!(body.get("top_logprobs").is_none());
        assert!(body.get("user").is_none());
        assert!(body.get("stream_options").is_none());
    }

    #[test]
    fn request_body_includes_required_tool_choice() {
        let options = OpenAiRequestOptions::default()
            .with_tool_choice(OpenAiToolChoice::Required);
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[sample_function_tool()],
            &options,
        );

        assert_eq!(body["tool_choice"], "required");
    }

    #[test]
    fn request_body_omits_tool_choice_when_tools_empty() {
        let options = OpenAiRequestOptions::default()
            .with_tool_choice(OpenAiToolChoice::Required)
            .with_parallel_tool_calls(true);
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[],
            &options,
        );

        assert!(
            body.get("tool_choice").is_none(),
            "tool_choice should not be sent when tools is empty"
        );
        assert!(
            body.get("parallel_tool_calls").is_none(),
            "parallel_tool_calls should not be sent \
            when tools is empty"
        );
    }

    #[test]
    fn request_body_includes_json_schema_response_format() {
        let options = OpenAiRequestOptions::default().with_response_format(
            OpenAiResponseFormat::json_schema(
                "tool_output".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "answer": { "type": "string" }
                    },
                    "required": ["answer"],
                    "additionalProperties": false
                }),
                true,
            ),
        );
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[],
            &options,
        );

        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(
            body["response_format"]["json_schema"]["name"],
            "tool_output"
        );
        assert_eq!(
            body["response_format"]["json_schema"]["schema"]["properties"]["answer"]["type"],
            "string"
        );
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
    }

    #[test]
    fn request_body_includes_sampling_and_token_fields() {
        let options = OpenAiRequestOptions::default()
            .with_temperature(0.2)
            .with_top_p(0.9)
            .with_max_completion_tokens(256);
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[],
            &options,
        );

        let temperature = body["temperature"]
            .as_f64()
            .expect("temperature should be encoded as a number");
        let top_p = body["top_p"]
            .as_f64()
            .expect("top_p should be encoded as a number");
        assert!((temperature - 0.2).abs() < 1e-6);
        assert!((top_p - 0.9).abs() < 1e-6);
        assert_eq!(body["max_completion_tokens"], 256);
    }

    #[test]
    fn request_body_includes_single_stop_sequence() {
        let options = OpenAiRequestOptions::default().with_stop(OpenAiStop::Single("END".into()));
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[],
            &options,
        );

        assert_eq!(body["stop"], "END");
    }

    #[test]
    fn request_body_includes_multiple_stop_sequences() {
        let options = OpenAiRequestOptions::default().with_stop(OpenAiStop::Multiple(vec![
            "END".to_string(),
            "STOP".to_string(),
        ]));
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[],
            &options,
        );

        assert_eq!(body["stop"], serde_json::json!(["END", "STOP"]));
    }

    #[test]
    fn request_body_includes_seed() {
        let options = OpenAiRequestOptions::default()
            .with_seed(12345);
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[],
            &options,
        );

        assert_eq!(body["seed"], 12345);
    }

    #[test]
    fn request_body_includes_parallel_tool_calls() {
        let options = OpenAiRequestOptions::default()
            .with_parallel_tool_calls(true);
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[sample_function_tool()],
            &options,
        );

        assert_eq!(body["parallel_tool_calls"], true);
    }

    #[test]
    fn request_body_includes_penalty_fields() {
        let options = OpenAiRequestOptions::default()
            .with_presence_penalty(0.3)
            .with_frequency_penalty(0.4);
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[],
            &options,
        );

        let presence_penalty = body["presence_penalty"]
            .as_f64()
            .expect("presence_penalty should be encoded as a number");
        let frequency_penalty = body["frequency_penalty"]
            .as_f64()
            .expect("frequency_penalty should be encoded as a number");
        assert!((presence_penalty - 0.3).abs() < 1e-6);
        assert!((frequency_penalty - 0.4).abs() < 1e-6);
    }

    #[test]
    fn request_body_includes_logprobs_fields() {
        let options = OpenAiRequestOptions::default()
            .with_logprobs(true)
            .with_top_logprobs(5);
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[],
            &options,
        );

        assert_eq!(body["logprobs"], true);
        assert_eq!(body["top_logprobs"], 5);
    }

    #[test]
    fn request_body_includes_user_field() {
        let options = OpenAiRequestOptions::default()
            .with_user("user-123".to_string());
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[],
            &options,
        );

        assert_eq!(body["user"], "user-123");
    }

    #[test]
    fn request_body_includes_stream_options() {
        let options = OpenAiRequestOptions::default()
            .with_stream_options(OpenAiStreamOptions {
                include_usage: Some(true),
            });
        let body = build_chat_completions_request_body(
            "gpt-test",
            &[sample_user_message()],
            &[],
            &options,
        );

        assert_eq!(
            body["stream_options"]["include_usage"],
            true
        );
    }
}
