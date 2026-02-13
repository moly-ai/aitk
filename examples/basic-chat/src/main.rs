use aitk::prelude::*;
use futures::StreamExt;

#[tokio::main]
async fn main() {
    let url = std::env::var("API_URL").expect("API_URL must be set");
    let key = std::env::var("API_KEY").expect("API_KEY must be set");
    let model = std::env::var("MODEL_ID").expect("MODEL_ID must be set");

    let mut client = OpenAiClient::new(url);
    client.set_key(&key).expect("Invalid API key");

    let bot_id = BotId::new(&model);
    let messages = vec![Message {
        from: EntityId::User,
        content: MessageContent {
            text: "Explain ownership in Rust in a few sentences.".into(),
            ..Default::default()
        },
        ..Default::default()
    }];

    let mut stream = client.send(&bot_id, &messages, &[]);
    while let Some(result) = stream.next().await {
        match result.into_result() {
            Ok(content) => print!("{}", content.text),
            Err(errors) => {
                for e in errors {
                    eprintln!("Error: {e}");
                }
            }
        }
    }

    println!();
}
