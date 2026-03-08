## OpenAI Standardized Options

Runs two real API checks against an OpenAI-compatible endpoint:

1. Structured output with `response_format: json_schema`
2. Tool flow with `tool_choice: required` and `parallel_tool_calls`

It also sets and exercises:

- `temperature`
- `top_p`
- `max_completion_tokens`
- `stop`
- `seed`
- `presence_penalty`
- `frequency_penalty`

### Requirements

```shell
export API_URL="https://api.openai.com/v1"
export API_KEY="sk-proj-xxxxxxxxxxxxxxxxxxxxxxxx"
export MODEL_ID="gpt-4.1-nano"
```

### Run

```shell
cargo run --manifest-path examples/openai-standardized/Cargo.toml
```

### Expected Output

- `=== Structured Output Test ===` followed by JSON text containing an `"answer"` field.
- `=== Tool Call Test ===` followed by at least one printed tool call and a final answer.
