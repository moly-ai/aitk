//! Native Gemini API client implementation.

use crate::protocol::*;
use crate::utils::asynchronous::{BoxPlatformSendFuture, BoxPlatformSendStream};
use crate::utils::sse::parse_sse;
use async_stream::stream;
use reqwest::header::{HeaderMap, HeaderName};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, HashMap},
    str::FromStr,
    sync::{Arc, RwLock},
};
use url::Url;

#[derive(Clone, Debug)]
struct GeminiClientInner {
    url: String,
    headers: HeaderMap,
    client: reqwest::Client,
}

/// A native Gemini API client using `/models` and `:streamGenerateContent`.
#[derive(Debug)]
pub struct GeminiClient(Arc<RwLock<GeminiClientInner>>);

impl Clone for GeminiClient {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl GeminiClient {
    /// Creates a new Gemini client for the given API base URL.
    pub fn new(url: String) -> Self {
        let inner = GeminiClientInner {
            url,
            headers: HeaderMap::new(),
            client: crate::utils::http::default_client(),
        };
        Self(Arc::new(RwLock::new(inner)))
    }

    /// Sets a custom HTTP header used in all Gemini requests.
    pub fn set_header(&mut self, key: &str, value: &str) -> Result<(), &'static str> {
        let header_name = HeaderName::from_str(key).map_err(|_| "Invalid header name")?;
        let header_value = value.parse().map_err(|_| "Invalid header value")?;
        self.0
            .write()
            .expect("gemini client lock poisoned")
            .headers
            .insert(header_name, header_value);
        Ok(())
    }

    /// Sets the Gemini API key used for request authentication.
    pub fn set_key(&mut self, key: &str) -> Result<(), &'static str> {
        self.set_header("x-goog-api-key", key)
    }
}

#[derive(Debug, Deserialize)]
struct GeminiModelsResponse {
    #[serde(default)]
    models: Vec<GeminiModel>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiModel {
    name: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "supportedGenerationMethods")]
    #[serde(default)]
    supported_generation_methods: Vec<String>,
}

#[derive(Debug, Serialize)]
struct GeminiGenerateRequest {
    contents: Vec<GeminiContent>,
    #[serde(rename = "system_instruction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiSystemInstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiToolDeclarations>>,
}

#[derive(Debug, Serialize)]
struct GeminiToolDeclarations {
    #[serde(rename = "function_declarations")]
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Debug, Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    parameters: Value,
}

#[derive(Debug, Serialize)]
struct GeminiSystemInstruction {
    parts: Vec<GeminiTextPart>,
}

#[derive(Debug, Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiOutgoingPart>,
}

#[derive(Debug, Serialize)]
struct GeminiTextPart {
    text: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum GeminiOutgoingPart {
    Text(GeminiTextPart),
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCall,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: GeminiFunctionResponse,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GeminiFunctionCall {
    #[serde(default)]
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Serialize)]
struct GeminiFunctionResponse {
    name: String,
    response: Value,
}

#[derive(Debug, Deserialize)]
struct GeminiStreamEvent {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiCandidateContent>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidateContent {
    #[serde(default)]
    parts: Vec<GeminiStreamPart>,
}

#[derive(Debug, Deserialize)]
struct GeminiStreamPart {
    #[serde(default)]
    text: String,
    #[serde(rename = "functionCall")]
    function_call: Option<GeminiFunctionCall>,
}

#[derive(Debug, Default)]
struct GeminiStreamDelta {
    text: String,
    function_calls: Vec<GeminiFunctionCall>,
}

fn normalize_model_id(id: &str) -> &str {
    id.trim_start_matches("models/")
}

fn build_endpoint_url(
    base_url: &str,
    suffix: &str,
    extra_query: &[(&str, &str)],
) -> Result<String, ClientError> {
    let mut url = Url::parse(base_url).map_err(|error| {
        ClientError::new_with_source(
            ClientErrorKind::Format,
            format!("Invalid Gemini base URL: {base_url}"),
            Some(error),
        )
    })?;

    let base_path = url.path().trim_end_matches('/');
    let suffix = suffix.trim_start_matches('/');
    let path = format!("{base_path}/{suffix}");
    url.set_path(&path);

    {
        let mut query = url.query_pairs_mut();
        for (key, value) in extra_query {
            query.append_pair(key, value);
        }
    }

    Ok(url.to_string())
}

fn build_models_url(base_url: &str, page_token: Option<&str>) -> Result<String, ClientError> {
    match page_token {
        Some(token) => build_endpoint_url(base_url, "models", &[("pageToken", token)]),
        None => build_endpoint_url(base_url, "models", &[]),
    }
}

fn build_stream_url(base_url: &str, bot_id: &BotId) -> Result<String, ClientError> {
    let model_id = bot_id.id();
    let model_path = if model_id.contains('/') {
        model_id.to_string()
    } else {
        format!("models/{}", normalize_model_id(model_id))
    };
    let suffix = format!("{model_path}:streamGenerateContent");
    build_endpoint_url(base_url, &suffix, &[("alt", "sse")])
}

fn supports_generate_content(model: &GeminiModel) -> bool {
    model.supported_generation_methods.is_empty()
        || model
            .supported_generation_methods
            .iter()
            .any(|method| method == "generateContent")
}

fn derive_capabilities() -> BotCapabilities {
    BotCapabilities::new().with_capabilities([BotCapability::TextInput, BotCapability::ToolInput])
}

fn gemini_model_to_bot(model: &GeminiModel) -> Option<Bot> {
    if !supports_generate_content(model) {
        return None;
    }

    let normalized_id = normalize_model_id(&model.name);
    let name = model
        .display_name
        .clone()
        .unwrap_or_else(|| normalized_id.to_string());

    Some(Bot {
        id: BotId::new(normalized_id),
        name,
        avatar: EntityAvatar::from_first_grapheme(&model.name.to_uppercase())
            .unwrap_or_else(|| EntityAvatar::Text("?".into())),
        capabilities: derive_capabilities(),
    })
}

#[cfg(test)]
fn parse_models_response(payload: &str) -> Result<Vec<Bot>, ClientError> {
    let response: GeminiModelsResponse = serde_json::from_str(payload).map_err(|error| {
        ClientError::new_with_source(
            ClientErrorKind::Format,
            "Could not parse Gemini models response.".to_string(),
            Some(error),
        )
    })?;

    let bots = response
        .models
        .iter()
        .filter_map(gemini_model_to_bot)
        .collect();
    Ok(bots)
}

fn as_tool_parameters(schema: &Map<String, Value>) -> Value {
    if schema.is_empty() {
        return serde_json::json!({
            "type": "object",
            "properties": {}
        });
    }
    Value::Object(schema.clone())
}

fn as_gemini_tools(tools: &[Tool]) -> Option<Vec<GeminiToolDeclarations>> {
    if tools.is_empty() {
        return None;
    }

    let function_declarations = tools
        .iter()
        .map(|tool| GeminiFunctionDeclaration {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: as_tool_parameters(&tool.input_schema),
        })
        .collect::<Vec<_>>();

    Some(vec![GeminiToolDeclarations {
        function_declarations,
    }])
}

fn collect_tool_call_names(messages: &[Message]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for message in messages {
        for call in &message.content.tool_calls {
            names.insert(call.id.clone(), call.name.clone());
        }
    }
    names
}

fn parse_tool_result_payload(result: &ToolResult) -> Value {
    match serde_json::from_str::<Value>(&result.content) {
        Ok(Value::Object(mut object)) => {
            if result.is_error && !object.contains_key("is_error") {
                object.insert("is_error".to_string(), Value::Bool(true));
            }
            Value::Object(object)
        }
        Ok(other) => serde_json::json!({
            "content": other,
            "is_error": result.is_error,
        }),
        Err(_) => serde_json::json!({
            "content": result.content,
            "is_error": result.is_error,
        }),
    }
}

fn as_bot_parts(message: &Message) -> Vec<GeminiOutgoingPart> {
    let mut parts = Vec::new();

    if !message.content.text.is_empty() {
        parts.push(GeminiOutgoingPart::Text(GeminiTextPart {
            text: message.content.text.clone(),
        }));
    }

    for call in &message.content.tool_calls {
        parts.push(GeminiOutgoingPart::FunctionCall {
            function_call: GeminiFunctionCall {
                name: call.name.clone(),
                args: Value::Object(call.arguments.clone()),
            },
        });
    }

    parts
}

fn as_tool_parts(
    message: &Message,
    tool_call_names: &HashMap<String, String>,
) -> Vec<GeminiOutgoingPart> {
    let mut parts = Vec::new();

    for result in &message.content.tool_results {
        if let Some(name) = tool_call_names.get(&result.tool_call_id) {
            parts.push(GeminiOutgoingPart::FunctionResponse {
                function_response: GeminiFunctionResponse {
                    name: name.clone(),
                    response: parse_tool_result_payload(result),
                },
            });
        } else if !result.content.is_empty() {
            parts.push(GeminiOutgoingPart::Text(GeminiTextPart {
                text: result.content.clone(),
            }));
        }
    }

    if parts.is_empty() && !message.content.text.is_empty() {
        parts.push(GeminiOutgoingPart::Text(GeminiTextPart {
            text: message.content.text.clone(),
        }));
    }

    parts
}

fn build_generate_request(
    messages: &[Message],
    tools: &[Tool],
) -> Result<GeminiGenerateRequest, ClientError> {
    let mut contents = Vec::with_capacity(messages.len());
    let mut system_blocks: Vec<String> = Vec::new();
    let tool_call_names = collect_tool_call_names(messages);

    for message in messages {
        match &message.from {
            EntityId::User => {
                if !message.content.text.is_empty() {
                    contents.push(GeminiContent {
                        role: "user".to_string(),
                        parts: vec![GeminiOutgoingPart::Text(GeminiTextPart {
                            text: message.content.text.clone(),
                        })],
                    });
                }
            }
            EntityId::Tool => {
                let parts = as_tool_parts(message, &tool_call_names);
                if !parts.is_empty() {
                    contents.push(GeminiContent {
                        role: "user".to_string(),
                        parts,
                    });
                }
            }
            EntityId::System => {
                if !message.content.text.is_empty() {
                    system_blocks.push(message.content.text.clone());
                }
            }
            EntityId::Bot(_) => {
                let parts = as_bot_parts(message);
                if !parts.is_empty() {
                    contents.push(GeminiContent {
                        role: "model".to_string(),
                        parts,
                    });
                }
            }
            EntityId::App => {
                return Err(ClientError::new(
                    ClientErrorKind::Format,
                    "App messages cannot be sent to Gemini.".to_string(),
                ));
            }
        }
    }

    if contents.is_empty() {
        return Err(ClientError::new(
            ClientErrorKind::Format,
            "Gemini request has no conversation content.".to_string(),
        ));
    }

    let system_instruction = if system_blocks.is_empty() {
        None
    } else {
        Some(GeminiSystemInstruction {
            parts: vec![GeminiTextPart {
                text: system_blocks.join("\n\n"),
            }],
        })
    };

    Ok(GeminiGenerateRequest {
        contents,
        system_instruction,
        tools: as_gemini_tools(tools),
    })
}

fn parse_stream_delta(payload: &str) -> Result<GeminiStreamDelta, ClientError> {
    let event: GeminiStreamEvent = serde_json::from_str(payload).map_err(|error| {
        ClientError::new_with_source(
            ClientErrorKind::Format,
            "Could not parse Gemini stream event.".to_string(),
            Some(error),
        )
    })?;

    let mut delta = GeminiStreamDelta::default();

    for candidate in event.candidates {
        if let Some(content) = candidate.content {
            for part in content.parts {
                if !part.text.is_empty() {
                    delta.text.push_str(&part.text);
                }
                if let Some(function_call) = part.function_call {
                    if !function_call.name.is_empty() {
                        delta.function_calls.push(function_call);
                    }
                }
            }
        }
    }

    Ok(delta)
}

#[cfg(test)]
fn parse_stream_text(payload: &str) -> Result<String, ClientError> {
    Ok(parse_stream_delta(payload)?.text)
}

fn function_call_args_to_map(args: Value) -> Map<String, Value> {
    match args {
        Value::Object(args) => args,
        Value::Null => Map::new(),
        other => {
            let mut arguments = Map::new();
            arguments.insert("value".to_string(), other);
            arguments
        }
    }
}

impl BotClient for GeminiClient {
    fn bots(&mut self) -> BoxPlatformSendFuture<'static, ClientResult<Vec<Bot>>> {
        let inner = self.0.read().expect("gemini client lock poisoned").clone();

        Box::pin(async move {
            let mut all_bots = Vec::new();
            let mut page_token: Option<String> = None;

            loop {
                let url = match build_models_url(&inner.url, page_token.as_deref()) {
                    Ok(url) => url,
                    Err(error) => return error.into(),
                };

                let response = match inner
                    .client
                    .get(&url)
                    .headers(inner.headers.clone())
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(error) => {
                        return ClientError::new_with_source(
                            ClientErrorKind::Network,
                            format!(
                                "Could not send request to {url}. \
                                 Verify your connection and key."
                            ),
                            Some(error),
                        )
                        .into();
                    }
                };

                if !response.status().is_success() {
                    let status = response.status();
                    let details = response.text().await.unwrap_or_default();
                    return ClientError::new(
                        ClientErrorKind::Response,
                        format!(
                            "Gemini models request failed \
                             with status {status}."
                        ),
                    )
                    .with_details(details)
                    .into();
                }

                let payload = match response.text().await {
                    Ok(text) => text,
                    Err(error) => {
                        return ClientError::new_with_source(
                            ClientErrorKind::Format,
                            format!(
                                "Could not read Gemini models \
                                 response from {url}."
                            ),
                            Some(error),
                        )
                        .into();
                    }
                };

                let parsed: GeminiModelsResponse = match serde_json::from_str(&payload) {
                    Ok(r) => r,
                    Err(error) => {
                        return ClientError::new_with_source(
                            ClientErrorKind::Format,
                            "Could not parse Gemini models \
                                 response."
                                .to_string(),
                            Some(error),
                        )
                        .into();
                    }
                };

                let bots = parsed
                    .models
                    .iter()
                    .filter_map(gemini_model_to_bot)
                    .collect::<Vec<_>>();

                all_bots.extend(bots);

                match parsed.next_page_token {
                    Some(token) if !token.is_empty() => {
                        page_token = Some(token);
                    }
                    _ => break,
                }
            }

            ClientResult::new_ok(all_bots)
        })
    }

    fn send(
        &mut self,
        bot_id: &BotId,
        messages: &[Message],
        tools: &[Tool],
    ) -> BoxPlatformSendStream<'static, ClientResult<MessageContent>> {
        let inner = self.0.read().expect("gemini client lock poisoned").clone();
        let bot_id = bot_id.clone();
        let messages = messages.to_vec();
        let tools = tools.to_vec();

        let stream = stream! {
            let url = match build_stream_url(&inner.url, &bot_id) {
                Ok(url) => url,
                Err(error) => {
                    yield error.into();
                    return;
                }
            };

            let request = match build_generate_request(&messages, &tools) {
                Ok(request) => request,
                Err(error) => {
                    yield error.into();
                    return;
                }
            };

            let response = match inner
                .client
                .post(&url)
                .headers(inner.headers)
                .json(&request)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    yield ClientError::new_with_source(
                        ClientErrorKind::Network,
                        format!(
                            "Could not send request to {url}. Verify your connection and key."
                        ),
                        Some(error),
                    ).into();
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let details = response.text().await.unwrap_or_default();
                yield ClientError::new(
                    ClientErrorKind::Response,
                    format!("Gemini streaming request failed with status {status}."),
                ).with_details(details).into();
                return;
            }

            let mut full_text = String::new();
            let mut tool_call_ids_by_index: HashMap<usize, String> = HashMap::new();
            let mut tool_calls_by_index: BTreeMap<usize, ToolCall> = BTreeMap::new();
            let mut next_tool_call_id = 0usize;
            let events = parse_sse(response.bytes_stream());

            for await event in events {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        yield ClientError::new_with_source(
                            ClientErrorKind::Network,
                            format!("Gemini response stream from {url} was interrupted."),
                            Some(error),
                        ).into();
                        return;
                    }
                };

                let delta = match parse_stream_delta(&event) {
                    Ok(delta) => delta,
                    Err(error) => {
                        yield error.into();
                        return;
                    }
                };

                if delta.text.is_empty() && delta.function_calls.is_empty() {
                    continue;
                }

                if !delta.text.is_empty() {
                    full_text.push_str(&delta.text);
                }

                for (index, function_call) in delta.function_calls.into_iter().enumerate() {
                    let call_id = tool_call_ids_by_index
                        .entry(index)
                        .or_insert_with(|| {
                            let call_id = format!("gemini-call-{next_tool_call_id}");
                            next_tool_call_id += 1;
                            call_id
                        })
                        .clone();

                    tool_calls_by_index.insert(
                        index,
                        ToolCall {
                            id: call_id,
                            name: function_call.name,
                            arguments: function_call_args_to_map(function_call.args),
                            ..Default::default()
                        },
                    );
                }

                let content = MessageContent {
                    text: full_text.clone(),
                    tool_calls: tool_calls_by_index.values().cloned().collect(),
                    ..Default::default()
                };
                yield ClientResult::new_ok(content);
            }
        };

        Box::pin(stream)
    }

    fn clone_box(&self) -> Box<dyn BotClient> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_models_response_prefers_display_name() {
        let payload = r#"
        {
          "models": [
            {
              "name": "models/gemini-2.0-flash",
              "displayName": "Gemini 2.0 Flash",
              "supportedGenerationMethods": ["generateContent"]
            }
          ]
        }"#;

        let bots = parse_models_response(payload).expect("failed to parse models response");
        let bot = bots.first().expect("expected one bot");

        assert_eq!(bot.id.id(), "gemini-2.0-flash");
        assert_eq!(bot.name, "Gemini 2.0 Flash");
    }

    #[test]
    fn models_url_preserves_existing_query() {
        let url = build_models_url(
            "https://generativelanguage.googleapis.com/v1beta?alt=sse",
            None,
        )
        .expect("failed to build models url");

        assert!(url.contains("/models?"));
        assert!(url.contains("alt=sse"));
    }

    #[test]
    fn models_url_includes_page_token() {
        let url = build_models_url(
            "https://generativelanguage.googleapis.com/v1beta",
            Some("abc123"),
        )
        .expect("failed to build models url");

        assert!(url.contains("pageToken=abc123"));
    }

    #[test]
    fn stream_url_uses_stream_generate_content() {
        let url = build_stream_url(
            "https://generativelanguage.googleapis.com/v1beta",
            &BotId::new("models/gemini-2.0-flash"),
        )
        .expect("failed to build stream url");

        assert!(url.contains("/models/gemini-2.0-flash:streamGenerateContent"));
        assert!(url.contains("alt=sse"));
    }

    #[test]
    fn stream_url_keeps_qualified_resource_path() {
        let url = build_stream_url(
            "https://generativelanguage.googleapis.com/v1beta",
            &BotId::new("tunedModels/my-tuned-model"),
        )
        .expect("failed to build stream url");

        assert!(url.contains("/tunedModels/my-tuned-model:streamGenerateContent"));
        assert!(!url.contains("/models/tunedModels/my-tuned-model:streamGenerateContent"));
    }

    #[test]
    fn build_generate_request_maps_system_user_and_model_roles() {
        let messages = vec![
            Message {
                from: EntityId::System,
                content: MessageContent {
                    text: "You are helpful.".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            },
            Message {
                from: EntityId::User,
                content: MessageContent {
                    text: "Hi".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            },
            Message {
                from: EntityId::Bot(BotId::new("gemini-2.0-flash")),
                content: MessageContent {
                    text: "Hello".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            },
        ];

        let request = build_generate_request(&messages, &[]).expect("failed to build request");

        assert_eq!(request.contents.len(), 2);
        assert_eq!(request.contents[0].role, "user");
        assert_eq!(request.contents[1].role, "model");
        assert_eq!(
            request
                .system_instruction
                .expect("missing system instruction")
                .parts[0]
                .text,
            "You are helpful."
        );
    }

    #[test]
    fn parse_models_response_maps_capabilities_from_generation_methods() {
        let payload = r#"
        {
          "models": [
            {
              "name": "models/gemini-2.0-flash",
              "supportedGenerationMethods": ["generateContent"]
            },
            {
              "name": "models/text-embedding-004",
              "supportedGenerationMethods": ["embedContent"]
            }
          ]
        }"#;

        let bots = parse_models_response(payload).expect("failed to parse");
        assert_eq!(bots.len(), 1, "embedding model should be filtered out");

        let bot = &bots[0];
        assert!(bot.capabilities.has_capability(&BotCapability::TextInput));
        assert!(bot.capabilities.has_capability(&BotCapability::ToolInput));
    }

    #[test]
    fn parse_stream_text_collects_all_candidate_parts() {
        let payload = r#"
        {
          "candidates": [
            { "content": { "parts": [{"text":"Hello "}, {"text":"Gemini"}] } }
          ]
        }"#;

        let text = parse_stream_text(payload).expect("failed to parse stream payload");
        assert_eq!(text, "Hello Gemini");
    }

    #[test]
    fn build_generate_request_includes_tool_declarations() {
        let messages = vec![Message {
            from: EntityId::User,
            content: MessageContent {
                text: "What's the weather in Tokyo?".to_string(),
                ..Default::default()
            },
            ..Default::default()
        }];

        let tools = vec![Tool {
            name: "get_weather".to_string(),
            description: Some("Get weather for a city.".to_string()),
            input_schema: std::sync::Arc::new(
                serde_json::from_str(
                    r#"{
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" }
                    },
                    "required": ["city"]
                }"#,
                )
                .expect("invalid schema json"),
            ),
        }];

        let request = build_generate_request(&messages, &tools).expect("failed to build request");
        let value = serde_json::to_value(request).expect("failed to serialize request");
        let declarations = value["tools"][0]["function_declarations"]
            .as_array()
            .expect("missing function_declarations");

        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0]["name"], "get_weather");
        assert_eq!(declarations[0]["parameters"]["type"], "object");
    }

    #[test]
    fn build_generate_request_maps_tool_results_to_function_response_parts() {
        let tool_call_id = "call-1".to_string();
        let messages = vec![
            Message {
                from: EntityId::Bot(BotId::new("gemini-2.0-flash")),
                content: MessageContent {
                    tool_calls: vec![ToolCall {
                        id: tool_call_id.clone(),
                        name: "filesystem__read_file".to_string(),
                        arguments: serde_json::Map::new(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
            Message {
                from: EntityId::Tool,
                content: MessageContent {
                    tool_results: vec![ToolResult {
                        tool_call_id,
                        content: r#"{"content":"hello"}"#.to_string(),
                        is_error: false,
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
        ];

        let request = build_generate_request(&messages, &[]).expect("failed to build request");
        let value = serde_json::to_value(request).expect("failed to serialize request");

        let model_parts = value["contents"][0]["parts"]
            .as_array()
            .expect("missing model parts");
        let tool_parts = value["contents"][1]["parts"]
            .as_array()
            .expect("missing tool parts");

        assert_eq!(
            model_parts[0]["functionCall"]["name"],
            "filesystem__read_file"
        );
        assert_eq!(
            tool_parts[0]["functionResponse"]["name"],
            "filesystem__read_file"
        );
    }

    #[test]
    fn parse_stream_delta_extracts_text_and_function_calls() {
        let payload = r#"
        {
          "candidates": [
            {
              "content": {
                "parts": [
                  {"text":"Checking..."},
                  {"functionCall":{"name":"get_weather","args":{"city":"Tokyo"}}}
                ]
              }
            }
          ]
        }"#;

        let delta = parse_stream_delta(payload).expect("failed to parse stream payload");
        assert_eq!(delta.text, "Checking...");
        assert_eq!(delta.function_calls.len(), 1);
        assert_eq!(delta.function_calls[0].name, "get_weather");
        assert_eq!(delta.function_calls[0].args["city"], "Tokyo");
    }
}
