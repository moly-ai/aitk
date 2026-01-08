use reqwest::StatusCode;

pub fn enrich_http_error(status: StatusCode, original: &str, body: Option<&str>) -> String {
    let clarification = match status {
        StatusCode::TOO_MANY_REQUESTS => {
            "This usually means you've hit a rate limit, run out of quota/credits, or do not have access to this resource/model in your current plan."
        }
        StatusCode::UNAUTHORIZED => "This usually means your API key is invalid or expired.",
        StatusCode::FORBIDDEN => {
            "This usually means you do not have permission to access this resource."
        }
        StatusCode::BAD_REQUEST => {
            "This might be an error on our side. If the problem persists, please file an issue on GitHub."
        }
        x if x >= StatusCode::INTERNAL_SERVER_ERROR
            && x <= StatusCode::HTTP_VERSION_NOT_SUPPORTED =>
        {
            "A server error occurred. This is likely a temporary issue with the provider."
        }
        _ => "",
    };

    let mut result = original.to_string();

    if let Some(body) = body {
        if !body.trim().is_empty() {
            result.push_str(&format!("\n\nResponse: {}", body));
        }
    }

    if !clarification.is_empty() {
        result.push_str(&format!("\n\nNote: {}", clarification));
    }

    result
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn default_client() -> reqwest::Client {
    use std::time::Duration;

    // On native, there are no default timeouts. Connection may hang if we don't
    // configure them.
    reqwest::Client::builder()
        // Only considered while establishing the connection.
        .connect_timeout(Duration::from_secs(90))
        // Considered while reading the response and reset on every chunk
        // received.
        //
        // Warning: Do not use normal `timeout` method as it doesn't consider
        // this.
        .read_timeout(Duration::from_secs(90))
        .build()
        .unwrap()
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn default_client() -> reqwest::Client {
    // On web, reqwest timeouts are not configurable, but it uses the browser's
    // fetch API under the hood, which handles connection issues properly.
    reqwest::Client::new()
}
