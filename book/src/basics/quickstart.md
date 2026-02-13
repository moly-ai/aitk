# Quickstart

This guide will get you from zero to a working AI call in a few minutes.

## Installation

aitk is not yet published on crates.io. Add it as a Git dependency:

```toml
[dependencies]
aitk = { git = "https://github.com/moly-chat/aitk.git", features = ["api-clients", "async-rt"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
futures = "0.3"
```

The `api-clients` feature enables the built-in HTTP clients. The `async-rt` feature gives you
the cross-platform `spawn()` utility, though you can use `tokio` directly if you prefer.

## Hello World

This example sends a message to an OpenAI-compatible API and prints the streamed response:

```rust
use aitk::prelude::*;
use futures::StreamExt;

#[tokio::main]
async fn main() {
    // Create and configure a client.
    let mut client = OpenAiClient::new("https://api.openai.com/v1".into());
    client.set_key("your-api-key".into());

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
    while let Some(result) = stream.next().await {
        if let Some(content) = result.into_value() {
            print!("{}", content.text);
        }
    }
    println!();
}
```

### What is happening here?

1. **`OpenAiClient::new(url)`** creates a client pointing at any OpenAI-compatible endpoint.
   This works with OpenAI, Azure, Ollama, LM Studio, or any provider that speaks the same
   protocol.

2. **`client.set_key(key)`** sets the API key sent as a Bearer token.

3. **`BotId::new("gpt-4.1-nano")`** identifies the model. This is the model ID string your
   provider expects.

4. **`client.send(&bot_id, &messages, &[])`** returns a `Stream` of `ClientResult<MessageContent>`
   items. Each item is a chunk of the streamed response. The third argument is the list of tools
   (empty here).

5. **`result.into_value()`** extracts the `MessageContent` from the result. The streaming
   chunks accumulate, so later chunks contain the full text so far.

```admonish tip
The `aitk::utils::asynchronous::spawn()` function (available with the `async-rt` feature) is a
cross-platform alternative to `tokio::spawn()`. You don't need it for basic usage, but it works
seamlessly on both native and WASM targets -- useful if you are building a cross-platform
application.
```

## Next Steps

- Learn about the [built-in clients](../clients/openai.md) in detail.
- See how to [generate images](../clients/openai-image.md) or
  [transcribe audio](../clients/openai-stt.md).
- Compose multiple providers with the [Router Client](../clients/router.md).
- Build a full chat application with the [ChatController](../chat-app/simple.md).
