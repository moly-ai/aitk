use crate::protocol::*;
use crate::utils::asynchronous::{BoxPlatformSendFuture, BoxPlatformSendStream};
use crate::utils::sse::parse_sse;
use async_stream::stream;
use reqwest::header::{HeaderMap, HeaderName};
use serde::{Deserialize, Serialize};
use std::{
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
            .unwrap()
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
}

#[derive(Debug, Serialize)]
struct GeminiSystemInstruction {
    parts: Vec<GeminiTextPart>,
}

#[derive(Debug, Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiTextPart>,
}

#[derive(Debug, Serialize)]
struct GeminiTextPart {
    text: String,
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

fn build_models_url(base_url: &str) -> Result<String, ClientError> {
    build_endpoint_url(base_url, "models", &[])
}

fn build_stream_url(
    base_url: &str,
    bot_id: &BotId,
) -> Result<String, ClientError> {
    let model_id = normalize_model_id(bot_id.id());
    let suffix = format!("models/{model_id}:streamGenerateContent");
    build_endpoint_url(base_url, &suffix, &[("alt", "sse")])
}

fn supports_generate_content(model: &GeminiModel) -> bool {
    model.supported_generation_methods.is_empty()
        || model
            .supported_generation_methods
            .iter()
            .any(|method| method == "generateContent")
}

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
        .filter(|model| supports_generate_content(model))
        .map(|model| {
            let normalized_id = normalize_model_id(&model.name);
            let name = model
                .display_name
                .clone()
                .unwrap_or_else(|| normalized_id.to_string());

            Bot {
                id: BotId::new(normalized_id),
                name,
                avatar: EntityAvatar::from_first_grapheme(&model.name.to_uppercase())
                    .unwrap_or_else(|| EntityAvatar::Text("?".into())),
                capabilities: BotCapabilities::new().with_capabilities([BotCapability::TextInput]),
            }
        })
        .collect();

    Ok(bots)
}

fn message_text(message: &Message) -> String {
    if !message.content.text.is_empty() {
        return message.content.text.clone();
    }

    if message.content.tool_results.is_empty() {
        return String::new();
    }

    message
        .content
        .tool_results
        .iter()
        .map(|result| result.content.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_generate_request(messages: &[Message]) -> Result<GeminiGenerateRequest, ClientError> {
    let mut contents = Vec::with_capacity(messages.len());
    let mut system_blocks: Vec<String> = Vec::new();

    for message in messages {
        let text = message_text(message);
        if text.is_empty() {
            continue;
        }

        match &message.from {
            EntityId::User | EntityId::Tool => contents.push(GeminiContent {
                role: "user".to_string(),
                parts: vec![GeminiTextPart { text }],
            }),
            EntityId::System => system_blocks.push(text),
            EntityId::Bot(_) => contents.push(GeminiContent {
                role: "model".to_string(),
                parts: vec![GeminiTextPart { text }],
            }),
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
    })
}

fn parse_stream_text(payload: &str) -> Result<String, ClientError> {
    let event: GeminiStreamEvent = serde_json::from_str(payload).map_err(|error| {
        ClientError::new_with_source(
            ClientErrorKind::Format,
            "Could not parse Gemini stream event.".to_string(),
            Some(error),
        )
    })?;

    let text = event
        .candidates
        .iter()
        .filter_map(|candidate| candidate.content.as_ref())
        .flat_map(|content| content.parts.iter())
        .map(|part| part.text.as_str())
        .collect::<Vec<_>>()
        .join("");

    Ok(text)
}

impl BotClient for GeminiClient {
    fn bots(&mut self) -> BoxPlatformSendFuture<'static, ClientResult<Vec<Bot>>> {
        let inner = self.0.read().unwrap().clone();

        Box::pin(async move {
            let url = match build_models_url(&inner.url) {
                Ok(url) => url,
                Err(error) => return error.into(),
            };

            let response = match inner.client.get(&url).headers(inner.headers).send().await {
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

            parse_models_response(&payload).into()
        })
    }

    fn send(
        &mut self,
        bot_id: &BotId,
        messages: &[Message],
        _tools: &[Tool],
    ) -> BoxPlatformSendStream<'static, ClientResult<MessageContent>> {
        let inner = self.0.read().unwrap().clone();
        let bot_id = bot_id.clone();
        let messages = messages.to_vec();

        let stream = stream! {
            let url = match build_stream_url(&inner.url, &bot_id) {
                Ok(url) => url,
                Err(error) => {
                    yield error.into();
                    return;
                }
            };

            let request = match build_generate_request(&messages) {
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

            let mut content = MessageContent::default();
            let mut full_text = String::new();
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

                let chunk = match parse_stream_text(&event) {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        yield error.into();
                        return;
                    }
                };

                if chunk.is_empty() {
                    continue;
                }

                full_text.push_str(&chunk);
                content.text = full_text.clone();
                yield ClientResult::new_ok(content.clone());
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
        )
        .expect("failed to build models url");

        assert!(url.contains("/models?"));
        assert!(url.contains("alt=sse"));
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

        let request = build_generate_request(&messages).expect("failed to build request");

        assert_eq!(request.contents.len(), 2);
        assert_eq!(request.contents[0].role, "user");
        assert_eq!(request.contents[1].role, "model");
        assert_eq!(
            request.system_instruction.expect("missing system instruction").parts[0].text,
            "You are helpful."
        );
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
}
