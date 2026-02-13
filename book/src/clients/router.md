# Router Client

`RouterClient` aggregates multiple `BotClient` implementations into a single client.
It routes requests to the correct sub-client based on a prefix in the `BotId`, letting
you work with models from different clients through one unified interface.

No additional feature flag is required -- `RouterClient` is always available.

## Creating a router

```rust
use aitk::prelude::*;

let router = RouterClient::new();
```

## Adding sub-clients

Each sub-client is registered under a string key. This key becomes the prefix used to
route requests:

```rust
let mut openai = OpenAiClient::new("https://api.openai.com/v1".into());
openai.set_key("your-openai-key".into());

let mut ollama = OpenAiClient::new("http://localhost:11434/v1".into());

router.insert_client("openai", Box::new(openai));
router.insert_client("ollama", Box::new(ollama));
```

## Bot ID prefixing

When you call `bots()` on a `RouterClient`, it fetches bots from all sub-clients and
prefixes each `BotId` with the sub-client's key and a `/` separator.

For example, if the `"openai"` sub-client reports a bot with ID `gpt-4.1`, the router
will expose it as `openai/gpt-4.1`.

If you forward bot IDs returned from `bots()` directly to `send()`, the routing is
automatic. If you construct `BotId`s manually, use the helper methods:

```rust
// Prefix manually.
let prefixed = RouterClient::prefix("openai", &BotId::new("gpt-4.1"));
assert_eq!(prefixed.as_str(), "openai/gpt-4.1");

// Unprefix to get the key and original ID.
let (key, original) = RouterClient::unprefix(&prefixed).unwrap();
assert_eq!(key, "openai");
assert_eq!(original.as_str(), "gpt-4.1");
```

## Caching

`RouterClient` caches the result of `bots()` for each sub-client. The cache is
populated on the first call and reused on subsequent calls, unless the cached result
contains errors (in which case it retries automatically).

To force a refresh:

```rust
// Invalidate all sub-clients.
router.invalidate_all_bots_cache();

// Or a specific one.
router.invalidate_bots_cache("openai");
```

## Accessing sub-clients

You can read or mutate a sub-client after registration:

```rust
// Immutable access.
router.read_client("openai", |client| {
    // Use `client` here.
});

// Mutable access.
router.write_client("openai", |client| {
    // Modify `client` here.
});

// Remove a sub-client entirely.
router.remove_client("ollama");
```

## Full example

```rust
use aitk::prelude::*;
use futures::StreamExt;

let router = RouterClient::new();

let mut openai = OpenAiClient::new("https://api.openai.com/v1".into());
openai.set_key("sk-...".into());
router.insert_client("openai", Box::new(openai));

let mut ollama = OpenAiClient::new("http://localhost:11434/v1".into());
router.insert_client("ollama", Box::new(ollama));

// List all models across both clients.
let mut router_clone = router.clone();
let result = router_clone.bots().await;
if let Some(bots) = result.value() {
    for bot in bots {
        println!("{}: {}", bot.id, bot.name);
        // e.g. "openai/gpt-4.1: GPT-4.1"
        // e.g. "ollama/llama3: Llama 3"
    }
}

// Send to a specific client's model.
let bot_id = BotId::new("openai/gpt-4.1-nano");
let messages = vec![Message {
    from: EntityId::User,
    content: MessageContent {
        text: "Hello!".into(),
        ..Default::default()
    },
    ..Default::default()
}];

let mut stream = router_clone.send(&bot_id, &messages, &[]);
while let Some(result) = stream.next().await {
    if let Some(content) = result.into_value() {
        print!("{}", content.text);
    }
}
```
