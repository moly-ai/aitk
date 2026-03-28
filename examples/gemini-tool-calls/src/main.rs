use aitk::prelude::*;
use futures::StreamExt;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let url = std::env::var("API_URL").expect("API_URL must be set");
    let key = std::env::var("API_KEY").expect("API_KEY must be set");
    let model = std::env::var("MODEL_ID").expect("MODEL_ID must be set");

    let mut client = GeminiClient::new(url);
    client.set_key(&key).expect("Invalid API key");

    let bot_id = BotId::new(&model);

    let weather_tool = Tool {
        name: "get_weather".into(),
        description: Some("Get current weather for a location".into()),
        input_schema: Arc::new(
            serde_json::from_str(
                r#"{
                    "type": "object",
                    "properties": {
                        "location": {
                            "type": "string",
                            "description": "City name, e.g. 'Tokyo'"
                        }
                    },
                    "required": ["location"]
                }"#,
            )
            .expect("Invalid JSON schema"),
        ),
    };

    let tools = [weather_tool];

    let mut messages = vec![Message {
        from: EntityId::User,
        content: MessageContent {
            text: "What's the weather like in Montevideo?".into(),
            ..Default::default()
        },
        ..Default::default()
    }];

    for turn in 0..5 {
        let assistant_content =
            match send_and_collect(&mut client, &bot_id, &messages, &tools).await {
                Ok(content) => content,
                Err(()) => return,
            };

        if assistant_content.tool_calls.is_empty() {
            println!("\nFinal answer:\n{}", assistant_content.text);
            return;
        }

        println!("\nTurn {} tool calls:", turn + 1);
        for tc in &assistant_content.tool_calls {
            println!("Tool call: {} with args {:?}", tc.name, tc.arguments);
        }

        messages.push(Message {
            from: EntityId::Bot(bot_id.clone()),
            content: assistant_content.clone(),
            ..Default::default()
        });

        for tc in &assistant_content.tool_calls {
            let result = execute_tool(tc);
            println!("Tool result for {}: {}", tc.name, result);

            messages.push(Message {
                from: EntityId::Tool,
                content: MessageContent {
                    tool_results: vec![ToolResult {
                        tool_call_id: tc.id.clone(),
                        content: result,
                        is_error: false,
                    }],
                    ..Default::default()
                },
                ..Default::default()
            });
        }
    }

    println!("\nReached max turns without final text.");
}

async fn send_and_collect(
    client: &mut GeminiClient,
    bot_id: &BotId,
    messages: &[Message],
    tools: &[Tool],
) -> Result<MessageContent, ()> {
    let mut last_content = MessageContent::default();
    let mut stream = client.send(bot_id, messages, tools);

    while let Some(result) = stream.next().await {
        match result.into_result() {
            Ok(content) => last_content = content,
            Err(errors) => {
                for e in errors {
                    eprintln!("Error: {e}");
                    if let Some(details) = e.details() {
                        eprintln!("Details: {details}");
                    }
                }
                return Err(());
            }
        }
    }

    Ok(last_content)
}

fn execute_tool(tool_call: &ToolCall) -> String {
    match tool_call.name.as_str() {
        "get_weather" => {
            let location = tool_call
                .arguments
                .get("location")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            format!(r#"{{"location": "{location}", "temp": "22C", "condition": "sunny"}}"#)
        }
        other => format!(r#"{{"error": "Unknown tool: {other}"}}"#),
    }
}
