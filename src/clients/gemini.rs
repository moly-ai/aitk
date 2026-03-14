//! Native Gemini API client implementation.

use crate::protocol::*;
use crate::utils::asynchronous::{BoxPlatformSendFuture, BoxPlatformSendStream};
use crate::utils::sse::parse_sse;
use async_stream::stream;
use reqwest::header::{HeaderMap, HeaderName};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
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
    #[serde(rename = "systemInstruction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiSystemInstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiToolDeclarations>>,
    #[serde(rename = "toolConfig")]
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_config: Option<GeminiToolConfig>,
}

#[derive(Debug, Serialize)]
struct GeminiToolDeclarations {
    #[serde(rename = "functionDeclarations")]
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
struct GeminiToolConfig {
    #[serde(rename = "functionCallingConfig")]
    function_calling_config: GeminiFunctionCallingConfig,
}

#[derive(Debug, Serialize)]
struct GeminiFunctionCallingConfig {
    mode: String,
    #[serde(rename = "allowedFunctionNames")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    allowed_function_names: Vec<String>,
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
        #[serde(rename = "thoughtSignature")]
        #[serde(skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: GeminiFunctionResponse,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GeminiFunctionCall {
    // Server-assigned call id used for function call/result correlation.
    #[serde(default)]
    id: Option<String>,
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
    #[serde(rename = "thoughtSignature")]
    thought_signature: Option<String>,
}

#[derive(Debug, Default)]
struct GeminiStreamDelta {
    text: String,
    function_calls: Vec<GeminiFunctionCallDelta>,
}

const TOOL_CALL_SIGNATURES_KEY: &str = "gemini_tool_call_thought_signatures";

#[derive(Debug)]
struct GeminiFunctionCallDelta {
    id: Option<String>,
    name: String,
    args: Value,
    thought_signature: Option<String>,
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

fn as_gemini_tool_config(tools: &[Tool]) -> Option<GeminiToolConfig> {
    if tools.is_empty() {
        return None;
    }

    Some(GeminiToolConfig {
        function_calling_config: GeminiFunctionCallingConfig {
            mode: "AUTO".to_string(),
            allowed_function_names: Vec::new(),
        },
    })
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
    let thought_signatures = parse_tool_call_thought_signatures(message.content.data.as_deref());

    if !message.content.text.is_empty() {
        parts.push(GeminiOutgoingPart::Text(GeminiTextPart {
            text: message.content.text.clone(),
        }));
    }

    for call in &message.content.tool_calls {
        parts.push(GeminiOutgoingPart::FunctionCall {
            function_call: GeminiFunctionCall {
                id: Some(call.id.clone()),
                name: call.name.clone(),
                args: Value::Object(call.arguments.clone()),
            },
            thought_signature: thought_signatures.get(&call.id).cloned(),
        });
    }

    parts
}

fn as_tool_parts(
    message: &Message,
    tool_call_names: &HashMap<String, String>,
) -> Result<Vec<GeminiOutgoingPart>, ClientError> {
    let mut parts = Vec::new();

    for result in &message.content.tool_results {
        if let Some(name) = tool_call_names.get(&result.tool_call_id) {
            parts.push(GeminiOutgoingPart::FunctionResponse {
                function_response: GeminiFunctionResponse {
                    name: name.clone(),
                    response: parse_tool_result_payload(result),
                },
            });
        } else {
            return Err(ClientError::new(
                ClientErrorKind::Format,
                format!(
                    "Gemini tool result references unknown tool call id '{}'.",
                    result.tool_call_id
                ),
            ));
        }
    }

    if parts.is_empty() && !message.content.text.is_empty() {
        parts.push(GeminiOutgoingPart::Text(GeminiTextPart {
            text: message.content.text.clone(),
        }));
    }

    Ok(parts)
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
                let parts = as_tool_parts(message, &tool_call_names)?;
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
        tool_config: as_gemini_tool_config(tools),
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
                if let Some(function_call) = part.function_call.filter(|c| !c.name.is_empty()) {
                    delta.function_calls.push(GeminiFunctionCallDelta {
                        id: function_call.id,
                        name: function_call.name,
                        args: function_call.args,
                        thought_signature: part.thought_signature,
                    });
                }
            }
        }
    }

    Ok(delta)
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

fn encode_tool_call_thought_signatures(signatures: &HashMap<String, String>) -> Option<String> {
    if signatures.is_empty() {
        return None;
    }

    let signatures_object = signatures
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect::<Map<String, Value>>();

    let mut root = Map::new();
    root.insert(
        TOOL_CALL_SIGNATURES_KEY.to_string(),
        Value::Object(signatures_object),
    );

    serde_json::to_string(&Value::Object(root)).ok()
}

fn parse_tool_call_thought_signatures(data: Option<&str>) -> HashMap<String, String> {
    let Some(data) = data else {
        return HashMap::new();
    };

    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return HashMap::new();
    };

    let Some(signatures) = value
        .as_object()
        .and_then(|root| root.get(TOOL_CALL_SIGNATURES_KEY))
        .and_then(Value::as_object)
    else {
        return HashMap::new();
    };

    signatures
        .iter()
        .filter_map(|(id, signature)| {
            signature
                .as_str()
                .map(|signature| (id.clone(), signature.to_string()))
        })
        .collect()
}

#[derive(Default)]
struct GeminiStreamToolCallState {
    by_stream_index: HashMap<usize, StreamToolCallSlot>,
    order: Vec<String>,
    calls_by_id: HashMap<String, ToolCall>,
    thought_signatures_by_id: HashMap<String, String>,
    next_id: usize,
}

struct StreamToolCallSlot {
    signature: String,
    id: String,
}

impl GeminiStreamToolCallState {
    fn apply_delta(&mut self, function_calls: Vec<GeminiFunctionCallDelta>) {
        for (stream_index, function_call) in function_calls.into_iter().enumerate() {
            let signature = stream_tool_call_signature(&function_call.name, &function_call.args);

            // Promotion policy for protocol ids:
            // 1. Same stream_index + same signature → promote (strongest match)
            // 2. Global signature search → promote only if exactly one candidate
            // 3. Multiple candidates → don't promote (ambiguous, prefer duplicate
            //    over wrong-assignment)
            let call_id = if let Some(protocol_id) = function_call.id.clone() {
                let matching_fallback = self
                    .by_stream_index
                    .get(&stream_index)
                    .filter(|slot| slot.signature == signature)
                    .map(|slot| (stream_index, slot.id.clone()))
                    .or_else(|| {
                        let candidates: Vec<_> = self
                            .by_stream_index
                            .iter()
                            .filter(|&(&idx, slot)| {
                                idx != stream_index && slot.signature == signature
                            })
                            .collect();
                        if candidates.len() == 1 {
                            let (&idx, slot) = candidates[0];
                            Some((idx, slot.id.clone()))
                        } else {
                            None
                        }
                    });

                self.by_stream_index.insert(
                    stream_index,
                    StreamToolCallSlot {
                        signature,
                        id: protocol_id.clone(),
                    },
                );

                if let Some((old_index, fallback_id)) = matching_fallback {
                    self.promote_call_id(&fallback_id, &protocol_id);
                    if old_index != stream_index {
                        self.by_stream_index.remove(&old_index);
                    }
                }

                self.ensure_ordered_id(&protocol_id);
                protocol_id
            } else {
                self.call_id_from_fallback(stream_index, signature)
            };

            self.calls_by_id.insert(
                call_id.clone(),
                ToolCall {
                    id: call_id.clone(),
                    name: function_call.name,
                    arguments: function_call_args_to_map(function_call.args),
                    ..Default::default()
                },
            );

            if let Some(thought_signature) = function_call.thought_signature {
                self.thought_signatures_by_id
                    .insert(call_id, thought_signature);
            }
        }
    }

    fn call_id_from_fallback(&mut self, stream_index: usize, signature: String) -> String {
        match self.by_stream_index.get(&stream_index) {
            Some(slot) if slot.signature == signature => slot.id.clone(),
            _ => {
                let id = format!("gemini-call-{}", self.next_id);
                self.next_id += 1;
                self.by_stream_index.insert(
                    stream_index,
                    StreamToolCallSlot {
                        signature,
                        id: id.clone(),
                    },
                );
                self.order.push(id.clone());
                id
            }
        }
    }

    fn promote_call_id(&mut self, previous_id: &str, protocol_id: &str) {
        if previous_id == protocol_id {
            return;
        }

        if let Some(mut call) = self.calls_by_id.remove(previous_id) {
            call.id = protocol_id.to_string();
            self.calls_by_id
                .entry(protocol_id.to_string())
                .or_insert(call);
        }

        if let Some(signature) = self.thought_signatures_by_id.remove(previous_id) {
            self.thought_signatures_by_id
                .entry(protocol_id.to_string())
                .or_insert(signature);
        }

        if let Some(pos) = self.order.iter().position(|id| id == previous_id) {
            self.order[pos] = protocol_id.to_string();
        }
    }

    fn ensure_ordered_id(&mut self, id: &str) {
        if self.order.iter().any(|existing| existing == id) {
            return;
        }
        self.order.push(id.to_string());
    }

    fn tool_calls(&self) -> Vec<ToolCall> {
        self.order
            .iter()
            .filter_map(|id| self.calls_by_id.get(id).cloned())
            .collect()
    }

    fn encoded_thought_signatures(&self) -> Option<String> {
        encode_tool_call_thought_signatures(&self.thought_signatures_by_id)
    }
}

fn stream_tool_call_signature(name: &str, args: &Value) -> String {
    let serialized_args = serde_json::to_string(args)
        .expect("serializing Gemini tool call arguments should not fail");
    format!("{name}:{serialized_args}")
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
                                "Could not send request to {url}. Verify your connection and key."
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
                        format!("Gemini models request failed with status {status}."),
                    )
                    .with_details(details)
                    .into();
                }

                let payload = match response.text().await {
                    Ok(text) => text,
                    Err(error) => {
                        return ClientError::new_with_source(
                            ClientErrorKind::Format,
                            format!("Could not read Gemini models response from {url}."),
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
                            "Could not parse Gemini models response.".to_string(),
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
            let mut stream_tool_call_state = GeminiStreamToolCallState::default();
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

                stream_tool_call_state.apply_delta(delta.function_calls);

                let content = MessageContent {
                    text: full_text.clone(),
                    tool_calls: stream_tool_call_state.tool_calls(),
                    data: stream_tool_call_state.encoded_thought_signatures(),
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
    fn models_use_display_name() {
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
    fn stream_url_qualified_path() {
        let url = build_stream_url(
            "https://generativelanguage.googleapis.com/v1beta",
            &BotId::new("tunedModels/my-tuned-model"),
        )
        .expect("failed to build stream url");

        assert!(url.contains("/tunedModels/my-tuned-model:streamGenerateContent"));
        assert!(!url.contains("/models/tunedModels/my-tuned-model:streamGenerateContent"));
    }

    #[test]
    fn request_maps_roles() {
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
                .as_ref()
                .expect("missing system instruction")
                .parts[0]
                .text,
            "You are helpful."
        );

        let value = serde_json::to_value(request).expect("failed to serialize request");
        assert_eq!(
            value["systemInstruction"]["parts"][0]["text"],
            "You are helpful."
        );
        assert!(
            value["system_instruction"].is_null(),
            "snake_case field should not be present"
        );
    }

    #[test]
    fn request_includes_tools() {
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
        let declarations = value["tools"][0]["functionDeclarations"]
            .as_array()
            .expect("missing function_declarations");

        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0]["name"], "get_weather");
        assert_eq!(declarations[0]["parameters"]["type"], "object");
        assert!(
            value["tools"][0]["function_declarations"].is_null(),
            "snake_case field should not be present"
        );
        assert_eq!(value["toolConfig"]["functionCallingConfig"]["mode"], "AUTO");
        assert!(
            value["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"].is_null(),
            "allowedFunctionNames should be omitted in AUTO mode"
        );
    }

    #[test]
    fn request_maps_tool_results() {
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
    fn request_rejects_unknown_tool_id() {
        let messages = vec![
            Message {
                from: EntityId::User,
                content: MessageContent {
                    text: "Use the tool".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            },
            Message {
                from: EntityId::Tool,
                content: MessageContent {
                    tool_results: vec![ToolResult {
                        tool_call_id: "missing-call".to_string(),
                        content: r#"{"ok":true}"#.to_string(),
                        is_error: false,
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
        ];

        let error = build_generate_request(&messages, &[])
            .expect_err("unknown tool result ids should fail request building");
        assert_eq!(error.kind(), ClientErrorKind::Format);
        assert!(
            error.to_string().contains("missing-call"),
            "error should mention the missing tool call id"
        );
    }

    #[test]
    fn delta_extracts_text_and_calls() {
        let payload = r#"
        {
          "candidates": [
            {
              "content": {
                "parts": [
                  {"text":"Checking..."},
                  {"functionCall":{"name":"get_weather","args":{"city":"Tokyo"}},"thoughtSignature":"sig-123"}
                ]
              }
            }
          ]
        }"#;

        let delta = parse_stream_delta(payload).expect("failed to parse stream payload");
        assert_eq!(delta.text, "Checking...");
        assert_eq!(delta.function_calls.len(), 1);
        assert_eq!(delta.function_calls[0].id, None);
        assert_eq!(delta.function_calls[0].name, "get_weather");
        assert_eq!(delta.function_calls[0].args["city"], "Tokyo");
        assert_eq!(
            delta.function_calls[0].thought_signature.as_deref(),
            Some("sig-123")
        );
    }

    #[test]
    fn tool_calls_distinct_across_chunks() {
        let mut state = GeminiStreamToolCallState::default();

        state.apply_delta(vec![GeminiFunctionCallDelta {
            id: None,
            name: "first_call".to_string(),
            args: serde_json::json!({"city":"Tokyo"}),
            thought_signature: None,
        }]);

        state.apply_delta(vec![GeminiFunctionCallDelta {
            id: None,
            name: "second_call".to_string(),
            args: serde_json::json!({"city":"Seoul"}),
            thought_signature: None,
        }]);

        let calls = state.tool_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "first_call");
        assert_eq!(calls[1].name, "second_call");
        assert_ne!(calls[0].id, calls[1].id);
    }

    #[test]
    fn tool_calls_prefer_protocol_id() {
        let payload = r#"
        {
          "candidates": [
            {
              "content": {
                "parts": [
                  {
                    "functionCall": {
                      "id": "protocol-call-42",
                      "name": "get_weather",
                      "args": {"city":"Tokyo"}
                    }
                  }
                ]
              }
            }
          ]
        }"#;

        let delta = parse_stream_delta(payload).expect("failed to parse stream payload");

        let mut state = GeminiStreamToolCallState::default();
        state.apply_delta(delta.function_calls);

        let calls = state.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "protocol-call-42");
        assert_eq!(calls[0].name, "get_weather");
    }

    #[test]
    fn tool_calls_upgrade_fallback_id() {
        let mut state = GeminiStreamToolCallState::default();

        state.apply_delta(vec![GeminiFunctionCallDelta {
            id: None,
            name: "get_weather".to_string(),
            args: serde_json::json!({"city":"Tokyo"}),
            thought_signature: Some("sig-local".to_string()),
        }]);

        state.apply_delta(vec![GeminiFunctionCallDelta {
            id: Some("protocol-call-42".to_string()),
            name: "get_weather".to_string(),
            args: serde_json::json!({"city":"Tokyo"}),
            thought_signature: Some("sig-protocol".to_string()),
        }]);

        let calls = state.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "protocol-call-42");
        assert_eq!(calls[0].name, "get_weather");

        let signatures =
            parse_tool_call_thought_signatures(state.encoded_thought_signatures().as_deref());
        assert_eq!(signatures.len(), 1);
        assert_eq!(
            signatures.get("protocol-call-42").map(String::as_str),
            Some("sig-protocol")
        );
    }

    #[test]
    fn tool_calls_survive_index_shift() {
        let mut state = GeminiStreamToolCallState::default();

        // Chunk 1: two calls without protocol ids at index 0 and 1.
        state.apply_delta(vec![
            GeminiFunctionCallDelta {
                id: None,
                name: "call_a".to_string(),
                args: serde_json::json!({"x": 1}),
                thought_signature: None,
            },
            GeminiFunctionCallDelta {
                id: None,
                name: "call_b".to_string(),
                args: serde_json::json!({"y": 2}),
                thought_signature: None,
            },
        ]);

        let calls = state.tool_calls();
        assert_eq!(calls.len(), 2);
        let a_fallback_id = calls[0].id.clone();
        let b_fallback_id = calls[1].id.clone();
        assert_eq!(calls[0].name, "call_a");
        assert_eq!(calls[1].name, "call_b");

        // Chunk 2: only call_b is resent, but now at index 0 with a protocol id.
        // The old index 0 held call_a — signatures differ, so call_a must NOT
        // be promoted to call_b's protocol id.
        state.apply_delta(vec![GeminiFunctionCallDelta {
            id: Some("proto-b".to_string()),
            name: "call_b".to_string(),
            args: serde_json::json!({"y": 2}),
            thought_signature: None,
        }]);

        let calls = state.tool_calls();
        assert_eq!(calls.len(), 2, "both calls must survive");

        let a = calls
            .iter()
            .find(|c| c.name == "call_a")
            .expect("call_a missing");
        let b = calls
            .iter()
            .find(|c| c.name == "call_b")
            .expect("call_b missing");

        assert_eq!(
            a.id, a_fallback_id,
            "call_a must keep its original fallback id"
        );
        assert_ne!(a.id, "proto-b", "call_a must NOT get call_b's protocol id");
        assert_eq!(b.id, "proto-b", "call_b must use its protocol id");
        assert_ne!(b.id, b_fallback_id, "call_b should have been upgraded");
    }

    #[test]
    fn tool_calls_no_promote_on_ambiguous_signature() {
        let mut state = GeminiStreamToolCallState::default();

        // Two calls with identical signature but different fallback ids.
        state.apply_delta(vec![
            GeminiFunctionCallDelta {
                id: None,
                name: "do_thing".to_string(),
                args: serde_json::json!({"x": 1}),
                thought_signature: None,
            },
            GeminiFunctionCallDelta {
                id: None,
                name: "do_thing".to_string(),
                args: serde_json::json!({"x": 1}),
                thought_signature: None,
            },
        ]);
        assert_eq!(state.tool_calls().len(), 2);

        // Protocol id arrives at a NEW index — two candidates match by signature,
        // so promotion must be skipped (ambiguous).
        state.apply_delta(vec![
            GeminiFunctionCallDelta {
                id: None,
                name: "other".to_string(),
                args: serde_json::json!({}),
                thought_signature: None,
            },
            GeminiFunctionCallDelta {
                id: None,
                name: "other".to_string(),
                args: serde_json::json!({}),
                thought_signature: None,
            },
            GeminiFunctionCallDelta {
                id: Some("proto-1".to_string()),
                name: "do_thing".to_string(),
                args: serde_json::json!({"x": 1}),
                thought_signature: None,
            },
        ]);

        let calls = state.tool_calls();
        // Both original fallback calls must survive untouched.
        let do_things: Vec<_> = calls.iter().filter(|c| c.name == "do_thing").collect();
        assert!(do_things.len() >= 2, "original fallback calls must not be consumed");
        assert!(
            do_things.iter().all(|c| c.id != "proto-1" || c.name == "do_thing"),
            "no fallback call should have been wrongly renamed"
        );
    }
}
