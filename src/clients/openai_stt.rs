//! Client based on the OpenAI one, but hits the speech-to-text API instead.

use crate::protocol::*;
use reqwest::header::{HeaderMap, HeaderName};
use std::{
    str::FromStr,
    sync::{Arc, RwLock},
};

#[derive(Debug, Clone)]
struct OpenAiSttClientInner {
    url: String,
    client: reqwest::Client,
    headers: HeaderMap,
}

/// Specific OpenAI client to hit speech-to-text endpoints.
#[derive(Debug)]
pub struct OpenAiSttClient(Arc<RwLock<OpenAiSttClientInner>>);

impl Clone for OpenAiSttClient {
    fn clone(&self) -> Self {
        OpenAiSttClient(Arc::clone(&self.0))
    }
}

impl OpenAiSttClient {
    pub fn new(url: String) -> Self {
        let headers = HeaderMap::new();
        let client = crate::utils::http::default_client();

        let inner = OpenAiSttClientInner {
            url,
            client,
            headers,
        };

        OpenAiSttClient(Arc::new(RwLock::new(inner)))
    }

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

    pub fn set_key(&mut self, key: &str) -> Result<(), &'static str> {
        self.set_header("Authorization", &format!("Bearer {}", key))
    }

    pub fn get_url(&self) -> String {
        self.0.read().unwrap().url.clone()
    }

    async fn transcribe_audio(
        &self,
        bot_id: &BotId,
        messages: &[Message],
    ) -> Result<MessageContent, ClientError> {
        let inner = self.0.read().unwrap().clone();

        let attachment = messages
            .last()
            .and_then(|msg| msg.content.attachments.first())
            .ok_or_else(|| {
                ClientError::new(
                    ClientErrorKind::Unknown,
                    "No audio attachment provided in the last message".to_string(),
                )
            })?;

        let bytes_arc = attachment.read().await.map_err(|e| {
            ClientError::new_with_source(
                ClientErrorKind::Unknown,
                format!("Failed to read attachment: {}", attachment.name),
                Some(e),
            )
        })?;
        let bytes = bytes_arc.to_vec();

        let file_part = reqwest::multipart::Part::bytes(bytes)
            .file_name(attachment.name.clone())
            .mime_str(
                attachment
                    .content_type
                    .as_deref()
                    .unwrap_or("application/octet-stream"),
            )
            .map_err(|e| {
                ClientError::new(
                    ClientErrorKind::Unknown,
                    format!("Invalid mime type for attachment: {}", e),
                )
            })?;

        let form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .text("model", bot_id.id().to_string());

        let url = format!("{}/audio/transcriptions", inner.url);

        let request = inner
            .client
            .post(&url)
            .headers(inner.headers.clone())
            .multipart(form);

        let response = request.send().await.map_err(|e| {
            ClientError::new_with_source(
                ClientErrorKind::Network,
                format!(
                    "Could not send request to {url}. Verify your connection and the server status."
                ),
                Some(e),
            )
        })?;

        let status = response.status();
        let text = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(ClientError::new(
                ClientErrorKind::Response,
                format!(
                    "Request to {url} failed with status {} and content: {}",
                    status, text
                ),
            ));
        }

        let response_json: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
            ClientError::new_with_source(
                ClientErrorKind::Format,
                format!(
                    "Failed to parse response from {url}. It does not match the expected format."
                ),
                Some(e),
            )
        })?;

        let transcript = response_json["text"].as_str().ok_or_else(|| {
            ClientError::new(
                ClientErrorKind::Format,
                format!("Response from {url} does not contain 'text' field."),
            )
        })?;

        let content = MessageContent {
            text: transcript.to_string(),
            ..Default::default()
        };

        Ok(content)
    }
}

impl BotClient for OpenAiSttClient {
    fn bots(&self) -> BoxPlatformSendFuture<'static, ClientResult<Vec<Bot>>> {
        let inner = self.0.read().unwrap().clone();

        // TODO: This is done in the image and realtime clients as well. But we
        // should stop doing this, as it makes the client less usable.
        // But is imposible to filter since capabilities are not exposed in the API.
        // Capabilities may not be something this crate should worry about.
        let supported: Vec<Bot> = ["whisper-1", "gpt-4o-transcribe", "gpt-4o-mini-transcribe"]
            .into_iter()
            .map(|id| Bot {
                id: BotId::new(id, &inner.url),
                name: id.to_string(),
                avatar: EntityAvatar::Text("W".into()),
                capabilities: BotCapabilities::new().with_capability(BotCapability::Attachments),
            })
            .collect();

        Box::pin(futures::future::ready(ClientResult::new_ok(supported)))
    }

    fn send(
        &mut self,
        bot_id: &BotId,
        messages: &[Message],
        _tools: &[Tool],
    ) -> BoxPlatformSendStream<'static, ClientResult<MessageContent>> {
        let self_clone = self.clone();
        let bot_id = bot_id.clone();
        let messages = messages.to_vec();

        Box::pin(async_stream::stream! {
            match self_clone.transcribe_audio(&bot_id, &messages).await {
                Ok(content) => yield ClientResult::new_ok(content),
                Err(e) => yield ClientResult::new_err(e.into()),
            }
        })
    }

    fn clone_box(&self) -> Box<dyn BotClient> {
        Box::new(self.clone())
    }
}
