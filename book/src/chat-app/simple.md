# Simple Usage

```admonish note
`ChatController` is entirely optional. You can build chat applications using the clients
directly -- the controller is simply a convenience for the common case. It provides
reusable business logic for managing conversation state, streaming responses, and loading
models, without tying you to any UI framework.
```

This chapter shows how to use it with a single model you already know the ID of.

**Required features:** `api-clients`, `async-rt`

## Overview

`ChatController` is held inside an `Arc<Mutex<...>>`. You interact with it by:

1. **Dispatching mutations** to change state (select a bot, push a message, etc.).
2. **Dispatching tasks** to trigger async operations (send messages, load models).
3. **Registering plugins** to get notified of state changes and integrate with your UI or
   other systems.

## Minimal example

```rust
use aitk::prelude::*;
use std::sync::{Arc, Mutex};

// 1. Create and configure a client.
let mut client = OpenAiClient::new("https://api.openai.com/v1".into());
client.set_key("your-api-key".into());

// 2. Build the controller.
let controller: Arc<Mutex<ChatController>> = ChatController::builder()
    .with_client(client)
    .with_basic_spawner()
    .build_arc();

// 3. Select a model.
controller
    .lock()
    .unwrap()
    .dispatch_mutation(ChatStateMutation::SetBotId(Some(BotId::new("gpt-4.1-nano"))));

// 4. Push a user message and send.
{
    let mut c = controller.lock().unwrap();
    c.dispatch_mutation(VecMutation::Push(Message {
        from: EntityId::User,
        content: MessageContent {
            text: "Hello!".into(),
            ..Default::default()
        },
        ..Default::default()
    }));
    c.dispatch_task(ChatTask::Send);
}
```

After calling `dispatch_task(ChatTask::Send)`, the controller:

1. Pushes an empty bot message (with `is_writing: true` metadata) into state.
2. Calls `send()` on the configured client with the full message history.
3. Updates the last message in state as streaming chunks arrive.
4. Clears the streaming flag when done.

## The builder

`ChatControllerBuilder` provides a fluent API for construction:

```rust
let controller = ChatController::builder()
    .with_client(client)           // Set the BotClient
    .with_basic_spawner()          // Cross-platform spawner (async-rt feature)
    .with_plugin_append(plugin)    // Register plugins
    .build_arc();                  // Produces Arc<Mutex<ChatController>>
```

The controller needs a spawner to run async tasks. `with_basic_spawner()` uses the
built-in cross-platform spawner from the `async-rt` feature, but you can provide any
custom spawner that implements the `Spawner` trait if you prefer a different runtime.

You can also create the controller manually and configure it step by step:

```rust
let controller = ChatController::new_arc();
{
    let mut c = controller.lock().unwrap();
    c.set_client(Some(Box::new(client)));
    c.set_basic_spawner();
    c.append_plugin(plugin);
}
```

## State

The controller's state is accessible via `controller.lock().unwrap().state()` and
contains:

| Field | Type | Description |
|---|---|---|
| `messages` | `Vec<Message>` | The full conversation history. |
| `bots` | `Vec<Bot>` | Models loaded from the client (see [Advanced Usage](advanced.md)). |
| `load_status` | `Status` | Status of the model loading operation. |
| `bot_id` | `Option<BotId>` | The model used when dispatching the `Send` task. |

## Mutations

State changes happen through `ChatStateMutation`:

```rust
// Select a bot.
c.dispatch_mutation(ChatStateMutation::SetBotId(Some(bot_id)));

// Push a message.
c.dispatch_mutation(VecMutation::Push(message));

// Clear messages.
c.dispatch_mutation(VecMutation::<Message>::Clear);
```

`VecMutation<Message>` and `VecMutation<Bot>` are automatically converted into
`ChatStateMutation` via `From`, so you can pass them directly to `dispatch_mutation`.

When multiple mutations should be applied as a batch, use `dispatch_mutations` with a
`Vec`. Individual `on_state_mutation` plugin hooks fire for each mutation, but
`on_state_ready` fires only once at the end of the batch.

## Tasks

`ChatTask` represents async operations:

| Task | Description |
|---|---|
| `Send` | Sends the current message history to the selected bot and streams the response. |
| `Stop` | Interrupts the current streaming operation. |
| `Load` | Fetches the list of available models from the client. |
| `Execute(tool_calls, bot_id)` | Executes MCP tool calls (requires the `mcp` feature). |

## Plugins

A plugin implements `ChatControllerPlugin` and is registered via `append_plugin` or
`prepend_plugin`. Plugins receive callbacks for state changes and can intercept tasks.

### Example: UI repaint trigger

The simplest plugin notifies your UI framework when state changes:

```rust
struct RepaintPlugin {
    // Your framework's handle for requesting repaints.
    ctx: UiContext,
}

impl ChatControllerPlugin for RepaintPlugin {
    fn on_state_ready(&mut self, _state: &ChatState, _mutations: &[ChatStateMutation]) {
        self.ctx.request_repaint();
    }
}
```

### Example: streaming text to stdout

A plugin that forwards streaming updates to the terminal:

```rust
use std::sync::mpsc::Sender;

struct CliPlugin {
    tx: Sender<String>,
}

impl ChatControllerPlugin for CliPlugin {
    fn on_state_mutation(&mut self, mutation: &ChatStateMutation, state: &ChatState) {
        let ChatStateMutation::MutateMessages(mutation) = mutation else {
            return;
        };

        for effect in mutation.effects(&state.messages) {
            if let VecEffect::Update(index, _old, new) = effect {
                if index == state.messages.len() - 1
                    && new.from != EntityId::User
                {
                    self.tx.send(new.content.text.clone()).unwrap();
                }
            }
        }
    }
}
```

### Plugin hooks

| Hook | When it fires |
|---|---|
| `on_state_ready(state, mutations)` | After all batched mutations are applied. Use this for UI updates. |
| `on_state_mutation(mutation, state)` | For each individual mutation, with the *pre-mutation* state. Useful for fine-grained change tracking. |
| `on_task(task) -> ChatControl` | Before a task executes. Return `ChatControl::Stop` to cancel it. |

## Reading state in a UI loop

A typical pattern in a GUI framework is to lock the controller, read its state, and
render:

```rust
let controller = controller.lock().unwrap();
let state = controller.state();

for message in &state.messages {
    match &message.from {
        EntityId::User => render_user_message(&message.content.text),
        EntityId::Bot(_) => render_bot_message(&message.content.text),
        _ => {}
    }
}
```

```admonish tip
If your UI framework allows it, prefer locking the controller once per render pass to
read all the state you need, rather than locking and unlocking repeatedly. This avoids
unnecessary contention and ensures a consistent view of the state for the entire frame.
```
