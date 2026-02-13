# Chat Completions

`OpenAiClient` is the primary client for streaming text completions from any
OpenAI-compatible API. It handles SSE streaming, attachments (images, PDFs, text files),
reasoning extraction, and tool calls.

**Feature flag:** `api-clients`

## Setup

```rust
use aitk::prelude::*;

let mut client = OpenAiClient::new("https://api.openai.com/v1".into());
client.set_key("your-api-key").unwrap();
```

The URL can point to any OpenAI-compatible endpoint: OpenAI, Azure, Ollama, LM Studio,
OpenRouter, and others.

### Custom headers

Some services require additional headers. Use `set_header` for these cases:

```rust
client.set_header("x-custom-header", "value").unwrap();
```

## Sending a message

The core interface is `send()`, which returns a stream of `ClientResult<MessageContent>` items.
Each item is a cumulative snapshot of the response built so far:

```rust
use futures::StreamExt;

let bot_id = BotId::new("gpt-4.1-nano");
let messages = vec![Message {
    from: EntityId::User,
    content: MessageContent {
        text: "Explain ownership in Rust.".into(),
        ..Default::default()
    },
    ..Default::default()
}];

let mut stream = client.send(&bot_id, &messages, &[]);
let mut last_content = MessageContent::default();

while let Some(result) = stream.next().await {
    match result.into_result() {
        Ok(content) => last_content = content,
        Err(errors) => {
            for e in errors {
                eprintln!("Error: {}", e);
            }
        }
    }
}

println!("{}", last_content.text);
```

### Multi-turn conversations

`send()` accepts the full message history. Include previous user and assistant messages
to maintain context:

```rust
let messages = vec![
    Message {
        from: EntityId::User,
        content: MessageContent {
            text: "What is Rust?".into(),
            ..Default::default()
        },
        ..Default::default()
    },
    Message {
        from: EntityId::Bot(BotId::new("gpt-4.1-nano")),
        content: MessageContent {
            text: "Rust is a systems programming language...".into(),
            ..Default::default()
        },
        ..Default::default()
    },
    Message {
        from: EntityId::User,
        content: MessageContent {
            text: "How does its borrow checker work?".into(),
            ..Default::default()
        },
        ..Default::default()
    },
];

let mut stream = client.send(&bot_id, &messages, &[]);
```

## Attachments

`OpenAiClient` automatically handles attachments included in a message. Images are sent as
`image_url` content parts, PDFs as file uploads, and text-based files (`.md`, `.html`, `.txt`,
etc.) are decoded and inlined.

```rust
let attachment = Attachment::from_bytes(
    "photo.png".into(),
    Some("image/png".into()),
    &image_bytes,
);

let messages = vec![Message {
    from: EntityId::User,
    content: MessageContent {
        text: "Describe this image.".into(),
        attachments: vec![attachment],
        ..Default::default()
    },
    ..Default::default()
}];
```

## Tool calls

To let the model invoke tools, pass a list of `Tool` definitions as the third argument
to `send()`. When the model decides to call a tool, the streamed `MessageContent` will
contain `tool_calls` instead of (or in addition to) text.

### Defining tools

```rust
use std::sync::Arc;

let tool = Tool {
    name: "get_weather".into(),
    description: Some("Get current weather for a location".into()),
    input_schema: Arc::new(serde_json::from_str(r#"{
        "type": "object",
        "properties": {
            "location": {
                "type": "string",
                "description": "City name"
            }
        },
        "required": ["location"]
    }"#).unwrap()),
};
```

### Handling tool calls and sending results

When the model returns tool calls, you execute the tools yourself and send the results
back in a follow-up message:

```rust
let mut stream = client.send(&bot_id, &messages, &[tool]);
while let Some(result) = stream.next().await {
    if let Some(content) = result.into_value() {
        if !content.tool_calls.is_empty() {
            // The model wants to call tools. Execute them and continue.
            for tc in &content.tool_calls {
                println!("Tool call: {} with args {:?}", tc.name, tc.arguments);
            }

            // Build a message with the tool results.
            let tool_result_message = Message {
                from: EntityId::Tool,
                content: MessageContent {
                    tool_results: vec![ToolResult {
                        tool_call_id: content.tool_calls[0].id.clone(),
                        content: r#"{"temp": "22C", "condition": "sunny"}"#.into(),
                        is_error: false,
                    }],
                    ..Default::default()
                },
                ..Default::default()
            };

            // Append to history and send again so the model can
            // produce a final answer using the tool output.
        }
    }
}
```

## Listing available models

All clients implement `bots()` which fetches the list of available models from the
configured endpoint:

```rust
let result = client.bots().await;
if let Some(bots) = result.value() {
    for bot in bots {
        println!("{}: {}", bot.id, bot.name);
    }
}
```

## Reasoning models

`OpenAiClient` automatically extracts reasoning/thinking content from models that support
it. The reasoning text is placed in `content.reasoning` and stripped from `content.text`.
This works with:

- Models that use `<think>` / `</think>` tags in the response text.
- Providers that return a `reasoning` or `reasoning_content` field in the streaming delta.
