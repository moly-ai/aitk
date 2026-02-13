# Advanced Usage

This chapter builds on the [simple usage](simple.md) guide and covers using
`ChatController` with a `RouterClient` for multiple clients and dynamic model loading.

## Router + ChatController

Instead of hardcoding a single client, you can use `RouterClient` to aggregate
multiple clients and let the user pick a model at runtime.

```rust
use aitk::prelude::*;

// Set up clients.
let mut openai = OpenAiClient::new("https://api.openai.com/v1".into());
openai.set_key("sk-...".into());

let ollama = OpenAiClient::new("http://localhost:11434/v1".into());

// Compose them into a router.
let router = RouterClient::new();
router.insert_client("openai", Box::new(openai));
router.insert_client("ollama", Box::new(ollama));

// Build the controller with the router as its client.
let controller = ChatController::builder()
    .with_client(router)
    .with_basic_spawner()
    .build_arc();
```

## Loading models dynamically

With a router (or any client that reports models), you can ask the controller to
fetch the list at runtime:

```rust
controller.lock().unwrap().dispatch_task(ChatTask::Load);
```

This triggers an async call to `bots()` on the configured client. When it completes,
the controller updates its state:

- `state.bots` contains the loaded models.
- `state.load_status` reflects the outcome (`Working`, `Success`, or `Error`).
- If there were errors, they appear as messages in `state.messages`.

You can then let the user select a model and set it:

```rust
let c = controller.lock().unwrap();
let state = c.state();

if state.load_status.is_success() {
    for bot in &state.bots {
        println!("{}: {}", bot.id, bot.name);
        // e.g. "openai/gpt-4.1: GPT-4.1"
    }
}
```

```rust
// After the user picks a model:
controller
    .lock()
    .unwrap()
    .dispatch_mutation(ChatStateMutation::SetBotId(Some(selected_bot_id)));
```

```admonish note
When using a `RouterClient`, the `BotId` values in `state.bots` are already prefixed
(e.g. `openai/gpt-4.1`). You can pass them directly to `SetBotId` without manual
prefixing.
```

## Swapping clients at runtime

You can replace the controller's client after construction. This resets the bot list
and load status:

```rust
let mut c = controller.lock().unwrap();
c.set_client(Some(Box::new(new_client)));
c.dispatch_task(ChatTask::Load);
```

## Refreshing the model list

If you are using a `RouterClient`, you can invalidate its cache and re-trigger a load:

```rust
{
    let c = controller.lock().unwrap();
    if let Some(client) = c.bot_client() {
        // Downcast or access the router if needed.
    }
}

// Or simply set the client again, which resets state and lets you reload.
```

The router caches `bots()` results per sub-client and only retries sub-clients whose
previous result contained errors. Call `invalidate_all_bots_cache()` on the router
directly if you need a full refresh.

## Intercepting tasks with plugins

A plugin can prevent a task from executing by returning `ChatControl::Stop` from
`on_task`. This is useful for adding confirmation dialogs or validation:

```rust
struct ConfirmPlugin;

impl ChatControllerPlugin for ConfirmPlugin {
    fn on_task(&mut self, task: &ChatTask) -> ChatControl {
        match task {
            ChatTask::Send => {
                // Add your validation logic here.
                // Return ChatControl::Stop to prevent sending.
                ChatControl::Continue
            }
            _ => ChatControl::Continue,
        }
    }
}
```

## Accessing sub-clients through the router

If you need to reconfigure a specific sub-client (for example, to update an API key),
you can access it through the router:

```rust
let mut c = controller.lock().unwrap();
if let Some(client) = c.bot_client_mut() {
    // If you know the client is a RouterClient, you can downcast and access sub-clients.
    // For simpler cases, consider holding a reference to the RouterClient directly.
}
```

A common pattern is to keep a clone of the `RouterClient` alongside the controller,
since `RouterClient` is `Clone` and internally reference-counted:

```rust
let router = RouterClient::new();
// ... insert sub-clients ...

let controller = ChatController::builder()
    .with_client(router.clone())
    .with_basic_spawner()
    .build_arc();

// Later, modify sub-clients through `router` directly.
router.write_client("openai", |client| {
    // Reconfigure the client.
});
router.invalidate_bots_cache("openai");
```
