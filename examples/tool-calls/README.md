## Tool Calls

Demonstrates tool use with `OpenAiClient`: defines a "get_weather" tool, handles the
model's tool call, executes it in Rust, and sends the result back for a final answer.

### Requirements

Set env variables and run:

```shell
export API_URL="https://api.openai.com/v1"
export API_KEY="sk-proj-xxxxxxxxxxxxxxxxxxxxxxxx"
export MODEL_ID="gpt-4.1-nano"
cargo run
```
