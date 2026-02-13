# Implementing a Custom Client

```admonish warning
This chapter is a placeholder. A detailed guide on implementing custom `BotClient`
implementations is coming soon.
```

All built-in clients implement the `BotClient` trait. You can implement it yourself to
support providers that don't follow the OpenAI-compatible API, or to add custom logic
around AI interactions.

## The BotClient trait

```rust
pub trait BotClient: Send {
    fn send(
        &mut self,
        bot_id: &BotId,
        messages: &[Message],
        tools: &[Tool],
    ) -> BoxPlatformSendStream<'static, ClientResult<MessageContent>>;

    fn bots(&mut self) -> BoxPlatformSendFuture<'static, ClientResult<Vec<Bot>>>;

    fn clone_box(&self) -> Box<dyn BotClient>;
}
```

- **`send()`** returns a stream of `ClientResult<MessageContent>`. Each yielded item
  should be a cumulative snapshot of the full response content built so far.
- **`bots()`** returns a future resolving to the list of available models.
- **`clone_box()`** enables `Box<dyn BotClient>` to be cloned.

Any custom client you implement will work with `RouterClient`, `ChatController`, and
every other abstraction in aitk that accepts a `BotClient`.
