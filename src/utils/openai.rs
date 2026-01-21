//! Shared definitions and utilities for the OpenAI spec.

use crate::protocol::*;
use serde::Deserialize;

/// A model from the models endpoint.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct Model {
    pub id: String,
}

/// Response from the models endpoint.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct Models {
    pub data: Vec<Model>,
}

#[cfg(feature = "api-clients")]
pub(crate) async fn get_models(
    client: &reqwest::Client,
    url: &str,
    headers: reqwest::header::HeaderMap,
) -> Result<Vec<Model>, ClientError> {
    let url = format!("{}/models", url);
    let request = client.get(&url).headers(headers);

    let response = request.send().await.map_err(|e| {
        ClientError::new_with_source(
            ClientErrorKind::Network,
            format!("An error ocurred sending a request to {url}."),
            Some(e),
        )
    })?;

    if !response.status().is_success() {
        let code = response.status().as_u16();
        return Err(ClientError::new(
            ClientErrorKind::Response,
            format!("Got unexpected HTTP status code {code} from {url}."),
        ));
    }

    let text = response.text().await.map_err(|e| {
        ClientError::new_with_source(
            ClientErrorKind::Format,
            format!("Could not parse the response from {url} as valid text."),
            Some(e),
        )
    })?;

    if text.is_empty() {
        return Err(ClientError::new(
            ClientErrorKind::Format,
            format!("The response from {url} is empty."),
        ));
    }

    let models: Models = serde_json::from_str(&text).map_err(|e| {
                    ClientError::new_with_source(
                        ClientErrorKind::Format,
                        format!("Could not parse the response from {url} as JSON or its structure does not match the expected format."),
                        Some(e),
                    )
                })?;

    Ok(models.data)
}

#[cfg(feature = "api-clients")]
pub(crate) async fn get_bots(
    client: &reqwest::Client,
    url: &str,
    headers: reqwest::header::HeaderMap,
    capabilities: &BotCapabilities,
) -> Result<Vec<Bot>, ClientError> {
    let models = get_models(client, url, headers).await?;

    let bots: Vec<Bot> = models
        .iter()
        .map(|m| Bot {
            id: BotId::new(&m.id),
            name: m.id.clone(),
            avatar: EntityAvatar::from_first_grapheme(&m.id.to_uppercase())
                .unwrap_or_else(|| EntityAvatar::Text("?".into())),
            capabilities: capabilities.clone(),
        })
        .collect();

    Ok(bots)
}
