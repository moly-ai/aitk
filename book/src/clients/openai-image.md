# Image Generation

`OpenAiImageClient` generates images using the OpenAI-compatible `/images/generations`
endpoint. It follows the same `BotClient` interface as all other clients: you send a
message and receive a `MessageContent` back, this time with an image `Attachment`.

**Feature flag:** `api-clients`

## Setup

```rust
use aitk::prelude::*;

let mut client = OpenAiImageClient::new("https://api.openai.com/v1".into());
client.set_key("your-api-key").unwrap();
```

## Generating an image

The prompt is taken from the `text` field of the last message. The response yields a
single `MessageContent` containing the generated image as an `Attachment`:

```rust
use futures::StreamExt;

let bot_id = BotId::new("dall-e-3");
let messages = vec![Message {
    from: EntityId::User,
    content: MessageContent {
        text: "A dragonfly perched on a blade of grass, watercolor style".into(),
        ..Default::default()
    },
    ..Default::default()
}];

let mut stream = client.send(&bot_id, &messages, &[]);
while let Some(result) = stream.next().await {
    if let Some(content) = result.into_value() {
        for attachment in &content.attachments {
            println!("Got image: {}", attachment.name);
            // Save the attachment to a file.
            attachment.save().await;
        }
    }
}
```

The client handles both base64 and URL responses transparently. In both cases the image
bytes are available through the `Attachment` API.

```admonish note
`Attachment` includes convenience methods like `save()` that interact directly with the
operating system (e.g. opening a save dialog on desktop, triggering a download on web).
These are provided for pragmatism but may change in a future version.
```
