# Introduction

**aitk** is a Rust crate that provides cross-platform, framework-agnostic abstractions for
working with AI models. It includes core types, traits, built-in API clients, and optional
state management utilities -- everything you need to integrate AI capabilities into any Rust
application.

## Features

- **Cross-platform**: works on native desktop (macOS, Windows, Linux), mobile (Android, iOS),
  and WebAssembly (`wasm32-unknown-unknown` -- no Emscripten, no WASI required).
- **Built-in API clients**: streaming chat completions, image generation, speech-to-text,
  realtime audio, and more to come.
- **Unified message format**: `MessageContent` can represent text, images, audio, tool calls,
  and other modalities in a single structure -- much like a traditional chat app that
  naturally handles mixed content. Every client speaks this same format through the
  `BotClient` trait, so switching or composing clients requires no changes to your
  application logic.
- **Composable**: the `RouterClient` lets you aggregate multiple clients under one interface,
  transparently routing requests to the right one.
- **MCP support**: discover and invoke tools from Model Context Protocol servers.
- **Flexible integration**: use the clients directly in a CLI tool, a GUI app, a web server, or
  anything else. There is no framework lock-in.
- **Optional state management**: if you are building a chat application, the `ChatController`
  provides business logic, streaming, model loading, and a plugin system -- without coupling to
  any UI framework.
- **Async & streaming**: built on standard Rust async patterns with `futures` streams, compatible
  with any async runtime.

## Feature Flags

aitk uses feature flags to let you include only what you need:

| Flag | Description |
|---|---|
| `api-clients` | Enables the built-in HTTP clients (`OpenAiClient`, `OpenAiImageClient`, `OpenAiSttClient`, etc.). Pulls in `reqwest`. |
| `realtime-clients` | Enables WebSocket-based clients (`OpenAiRealtimeClient`). Pulls in `tokio` and `tokio-tungstenite`. |
| `async-rt` | Includes `tokio` (native) and `wasm-bindgen-futures` (WASM), exposing a unified `spawn()` function. |
| `mcp` | Enables MCP tool integration. Implies `async-rt` and `api-clients`. |
| `full` | Enables everything above. |

With no features enabled, you get the core types and traits (`BotClient`, `Message`,
`MessageContent`, etc.) with zero heavy dependencies. This is useful if you want to implement
your own client without pulling in `reqwest` or `tokio`.

## Crate Organization

```
aitk
├── protocol   Core types and traits (BotClient, Message, Bot, Tool, ...)
├── clients    Built-in BotClient implementations
├── controllers  State management (ChatController)
├── mcp        Model Context Protocol integration (feature-gated)
└── utils      Cross-platform async primitives, helpers
```

A `prelude` module re-exports the most commonly used types:

```rust
use aitk::prelude::*;
```
