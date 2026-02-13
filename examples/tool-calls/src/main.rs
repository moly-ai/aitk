use aitk::prelude::*;
use futures::StreamExt;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let url = std::env::var("API_URL").expect("API_URL must be set");
    let key = std::env::var("API_KEY").expect("API_KEY must be set");
    let model = std::env::var("MODEL_ID").expect("MODEL_ID must be set");

    let mut client = OpenAiClient::new(url);
    client.set_key(&key).expect("Invalid API key");

    let bot_id = BotId::new(&model);

    // Define a tool the model can call.
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

    // Ask the model something that requires the tool.
    let mut messages = vec![Message {
        from: EntityId::User,
        content: MessageContent {
            text: "What's the weather like in Montevideo?".into(),
            ..Default::default()
        },
        ..Default::default()
    }];

    // First send: the model should respond with a tool call.
    let assistant_content = send_and_collect(&mut client, &bot_id, &messages, &tools).await;

    if assistant_content.tool_calls.is_empty() {
        println!("Model responded without tool calls: {}", assistant_content.text);
        return;
    }

    for tc in &assistant_content.tool_calls {
        println!("Tool call: {} with args {:?}", tc.name, tc.arguments);
    }

    // Append the assistant message (with tool calls) to the history.
    messages.push(Message {
        from: EntityId::Bot(bot_id.clone()),
        content: assistant_content.clone(),
        ..Default::default()
    });

    // Execute each tool call and build tool result messages.
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

    // Second send: the model uses the tool results to produce a final answer.
    println!("\nFinal answer:");
    let final_content = send_and_collect(&mut client, &bot_id, &messages, &tools).await;
    println!("{}", final_content.text);
}

/// Sends the message history and collects the final streamed snapshot.
async fn send_and_collect(
    client: &mut OpenAiClient,
    bot_id: &BotId,
    messages: &[Message],
    tools: &[Tool],
) -> MessageContent {
    let mut last_content = MessageContent::default();
    let mut stream = client.send(bot_id, messages, tools);

    while let Some(result) = stream.next().await {
        match result.into_result() {
            Ok(content) => last_content = content,
            Err(errors) => {
                for e in errors {
                    eprintln!("Error: {e}");
                }
            }
        }
    }

    last_content
}

/// A mock tool implementation. In a real application this could call an actual weather
/// API, query a database, run a calculation, etc.
fn execute_tool(tool_call: &ToolCall) -> String {
    match tool_call.name.as_str() {
        "get_weather" => {
            let location = tool_call
                .arguments
                .get("location")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            // Return a fake weather response as JSON.
            format!(
                r#"{{"location": "{location}", "temp": "22°C", "condition": "sunny"}}"#
            )
        }
        other => format!(r#"{{"error": "Unknown tool: {other}"}}"#),
    }
}
