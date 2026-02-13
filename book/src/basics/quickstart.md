# Quickstart

This guide will get you from zero to a working AI call in a few minutes.

## Installation

aitk is not yet published on crates.io. Add it as a Git dependency:

```toml
[dependencies]
aitk = { git = "https://github.com/moly-ai/aitk.git", features = ["api-clients"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
futures = "0.3"
```

The `api-clients` feature enables the built-in HTTP clients.

## Hello World

This example sends a message to an OpenAI-compatible API and prints the streamed response:

```rust
use aitk::prelude::*;
use futures::StreamExt;

#[tokio::main]
async fn main() {
    // Create and configure a client.
    let mut client = OpenAiClient::new("https://api.openai.com/v1".into());
    client.set_key("your-api-key").unwrap();

    // Build the request.
    let bot_id = BotId::new("gpt-4.1-nano");
    let messages = vec![Message {
        from: EntityId::User,
        content: MessageContent {
            text: "Hello! What is Rust?".into(),
            ..Default::default()
        },
        ..Default::default()
    }];

    // Send and stream the response.
    let mut stream = client.send(&bot_id, &messages, &[]);
    let mut last_content = MessageContent::default();
    
    while let Some(result) = stream.next().await {
        if let Some(content) = result.into_value() {
            last_content = content;
        }
    }
    
    println!("{}", last_content.text);
}
```

### What is happening here?

1. **`OpenAiClient::new(url)`** creates a client pointing at any OpenAI-compatible endpoint.
   This works with OpenAI, Azure, Ollama, LM Studio, or any service that speaks the same
   protocol.

2. **`client.set_key(key)`** sets the API key sent as a Bearer token.

3. **`BotId::new("gpt-4.1-nano")`** identifies the model. This is the model ID string your
   endpoint expects.

4. **`client.send(&bot_id, &messages, &[])`** returns a `Stream` of `ClientResult<MessageContent>`
   items. Each item is a cumulative snapshot of the full response built so far. The third
   argument is the list of tools (empty here).

5. **`result.into_value()`** extracts the `MessageContent` from the result.

## Next Steps

- Learn about the [built-in clients](../clients/openai.md) in detail.
- See how to [generate images](../clients/openai-image.md) or
  [transcribe audio](../clients/openai-stt.md).
- Compose multiple clients with the [Router Client](../clients/router.md).
- Build a full chat application with the [ChatController](../chat-app/simple.md).
