use aitk::clients::openai::{
    OpenAiResponseFormat, OpenAiStop, OpenAiToolChoice,
};
use aitk::prelude::*;
use futures::StreamExt;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let url = std::env::var("API_URL").expect("API_URL must be set");
    let key = std::env::var("API_KEY").expect("API_KEY must be set");
    let model = std::env::var("MODEL_ID").expect("MODEL_ID must be set");

    run_structured_output_test(&url, &key, &model).await;
    run_tool_call_test(&url, &key, &model).await;
}

async fn run_structured_output_test(url: &str, key: &str, model: &str) {
    println!("=== Structured Output Test ===");

    let mut client = OpenAiClient::new(url.to_string());
    client.set_key(key).expect("Invalid API key");
    client.set_temperature(Some(0.2));
    client.set_top_p(Some(0.9));
    client.set_max_completion_tokens(Some(256));
    client.set_seed(Some(42));
    client.set_presence_penalty(Some(0.2));
    client.set_frequency_penalty(Some(0.2));
    client.set_stop(Some(OpenAiStop::Single("<END>".to_string())));
    client.set_response_format(Some(OpenAiResponseFormat::json_schema(
        "answer_payload".to_string(),
        serde_json::json!({
            "type": "object",
            "properties": {
                "answer": { "type": "string" }
            },
            "required": ["answer"],
            "additionalProperties": false
        }),
        true,
    )));

    let bot_id = BotId::new(model);
    let messages = vec![Message {
        from: EntityId::User,
        content: MessageContent {
            text: "Return JSON only with field `answer`. \
                   Explain what ownership in Rust means in one sentence."
                .to_string(),
            ..Default::default()
        },
        ..Default::default()
    }];

    let content = send_and_collect(&mut client, &bot_id, &messages, &[]).await;
    println!("Raw response:\n{}", content.text);

    let parsed: serde_json::Value =
        serde_json::from_str(&content.text).expect("Expected valid JSON text");
    let answer = parsed
        .get("answer")
        .and_then(|v| v.as_str())
        .expect("Expected a string field named `answer`");
    println!("Parsed answer: {answer}");
}

async fn run_tool_call_test(url: &str, key: &str, model: &str) {
    println!("\n=== Tool Call Test ===");

    let mut client = OpenAiClient::new(url.to_string());
    client.set_key(key).expect("Invalid API key");
    client.set_temperature(Some(0.2));
    client.set_top_p(Some(0.9));
    client.set_max_completion_tokens(Some(256));
    client.set_seed(Some(42));
    client.set_tool_choice(Some(OpenAiToolChoice::Required));
    client.set_parallel_tool_calls(Some(true));
    client.set_response_format(None);
    client.set_stop(None);

    let bot_id = BotId::new(model);
    let tools = [weather_tool()];

    let mut messages = vec![Message {
        from: EntityId::User,
        content: MessageContent {
            text: "What is the weather in Montevideo? Use the tool.".to_string(),
            ..Default::default()
        },
        ..Default::default()
    }];

    let assistant_content = send_and_collect(&mut client, &bot_id, &messages, &tools).await;

    if assistant_content.tool_calls.is_empty() {
        panic!("Expected at least one tool call when tool_choice is required");
    }

    for tc in &assistant_content.tool_calls {
        println!("Tool call: {} args={:?}", tc.name, tc.arguments);
    }

    messages.push(Message {
        from: EntityId::Bot(bot_id.clone()),
        content: assistant_content.clone(),
        ..Default::default()
    });

    for tc in &assistant_content.tool_calls {
        let tool_result = execute_tool(tc);
        println!("Tool result for {}: {}", tc.name, tool_result);
        messages.push(Message {
            from: EntityId::Tool,
            content: MessageContent {
                tool_results: vec![ToolResult {
                    tool_call_id: tc.id.clone(),
                    content: tool_result,
                    is_error: false,
                }],
                ..Default::default()
            },
            ..Default::default()
        });
    }

    let final_content = send_and_collect(&mut client, &bot_id, &messages, &tools).await;
    println!("Final answer:\n{}", final_content.text);
}

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
                for error in errors {
                    eprintln!("Error: {error}");
                    if let Some(details) = error.details() {
                        eprintln!("Details: {details}");
                    }
                }
            }
        }
    }

    last_content
}

fn weather_tool() -> Tool {
    Tool {
        name: "get_weather".to_string(),
        description: Some("Get current weather for a city".to_string()),
        input_schema: Arc::new(
            serde_json::from_str(
                r#"{
                    "type": "object",
                    "properties": {
                        "location": {
                            "type": "string",
                            "description": "City name, e.g. 'Montevideo'"
                        }
                    },
                    "required": ["location"]
                }"#,
            )
            .expect("Tool schema must be a valid JSON object"),
        ),
    }
}

fn execute_tool(tool_call: &ToolCall) -> String {
    match tool_call.name.as_str() {
        "get_weather" => {
            let location = tool_call
                .arguments
                .get("location")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            format!(
                r#"{{"location":"{location}","temp":"22°C","condition":"sunny"}}"#
            )
        }
        other => format!(r#"{{"error":"Unknown tool: {other}"}}"#),
    }
}
