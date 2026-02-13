# Realtime

```admonish warning
This chapter is a placeholder. The realtime client documentation is coming soon.
```

`OpenAiRealtimeClient` provides WebSocket-based communication for real-time audio
interactions using the OpenAI Realtime API. It supports bidirectional audio streaming,
voice activity detection, and function calling during a live session.

**Feature flag:** `realtime-clients`

The realtime client differs from the other clients in that `send()` returns a
`MessageContent` containing an `Upgrade::Realtime` with channels for sending commands
and receiving events, rather than streaming text content directly.
