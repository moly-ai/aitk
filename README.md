# aitk

**aitk** is a Rust crate that provides cross-platform, framework-agnostic abstractions for working with AI models. It includes core types, traits, built-in API clients, and optional state management utilities—everything you need to integrate AI capabilities into any Rust application.

## Features

- **Cross-platform**: works on native desktop (macOS, Windows, Linux), mobile (Android, iOS), and WebAssembly (`wasm32-unknown-unknown` with no Emscripten, no WASI required).
- **Built-in API clients**: streaming chat completions, image generation, speech-to-text, realtime audio, and more to come.
- **Unified message format**: `MessageContent` can represent text, images, audio, tool calls, and other modalities in a single structure—much like a traditional chat app that naturally handles mixed content. Every client speaks this same format through the `BotClient` trait, so switching or composing clients requires no changes to your application logic.
- **Composable**: the `RouterClient` lets you aggregate multiple clients under one interface, transparently routing requests to the right one.
- **MCP support**: discover and invoke tools from Model Context Protocol servers.
- **Flexible integration**: use the clients directly in a CLI tool, a GUI app, a web server, or anything else. There is no framework lock-in.
- **Optional state management**: if you are building a chat application, the `ChatController` provides business logic, streaming, model loading, and a plugin system—without coupling to any UI framework.
- **Async & streaming**: built on standard Rust async patterns with `futures` streams, compatible with any async runtime.

## Documentation

For detailed documentation, please visit the [GitHub page](https://moly-ai.github.io/aitk).

## Examples

Check out the `examples/` directory for practical usage examples.
