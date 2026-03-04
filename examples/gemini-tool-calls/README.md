## Gemini Native Tool Calls

Demonstrates native Gemini tool-calling with `GeminiClient`:
- Send tool declarations (`function_declarations`)
- Receive model tool calls
- Execute tools in Rust
- Send `ToolResult` back to Gemini
- Receive final answer

### Requirements

Set env variables and run:

```shell
export API_URL="https://generativelanguage.googleapis.com/v1beta"
export API_KEY="your-gemini-key"
export MODEL_ID="gemini-2.0-flash"
cargo run
```
