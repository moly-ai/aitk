use aitk::prelude::*;
use futures::StreamExt;

#[tokio::main]
async fn main() {
    let url = std::env::var("API_URL").expect("API_URL must be set");
    let key = std::env::var("API_KEY").expect("API_KEY must be set");
    let model = std::env::var("MODEL_ID").expect("MODEL_ID must be set");

    let mut client = OpenAiImageClient::new(url);
    client.set_key(&key).expect("Invalid API key");

    let bot_id = BotId::new(&model);
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
        match result.into_result() {
            Ok(content) => {
                for attachment in &content.attachments {
                    println!("Generated image: {}", attachment.name);

                    // Read the raw bytes and write to disk.
                    match attachment.read().await {
                        Ok(bytes) => {
                            let path = &attachment.name;
                            std::fs::write(path, &*bytes)
                                .expect("Failed to write image file");
                            println!("Saved to {path}");
                        }
                        Err(e) => eprintln!("Failed to read attachment: {e}"),
                    }
                }
            }
            Err(errors) => {
                for e in errors {
                    eprintln!("Error: {e}");
                }
            }
        }
    }
}
