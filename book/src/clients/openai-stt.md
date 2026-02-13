# Speech-to-Text

`OpenAiSttClient` transcribes audio files using the OpenAI-compatible
`/audio/transcriptions` endpoint. Like all clients, it implements `BotClient` and uses
`send()` as its entry point.

**Feature flag:** `api-clients`

## Setup

```rust
use aitk::prelude::*;

let mut client = OpenAiSttClient::new("https://api.openai.com/v1".into());
client.set_key("your-api-key".into());
```

## Transcribing audio

The audio must be provided as an `Attachment` on the last message. The response text
contains the transcription:

```rust
use futures::StreamExt;

let audio_bytes: Vec<u8> = std::fs::read("recording.mp3").unwrap();
let attachment = Attachment::from_bytes(
    "recording.mp3".into(),
    Some("audio/mpeg".into()),
    &audio_bytes,
);

let bot_id = BotId::new("whisper-1");
let messages = vec![Message {
    from: EntityId::User,
    content: MessageContent {
        attachments: vec![attachment],
        ..Default::default()
    },
    ..Default::default()
}];

let mut stream = client.send(&bot_id, &messages, &[]);
while let Some(result) = stream.next().await {
    if let Some(content) = result.into_value() {
        println!("Transcription: {}", content.text);
    }
}
```

The client sends the audio as a multipart form upload with the model ID and the file.
