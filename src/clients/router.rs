use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use smol_str::SmolStr;

use crate::protocol::*;
use futures::StreamExt;

#[derive(Clone)]
struct Item {
    client: Box<dyn BotClient>,
    bots_result: Option<ClientResult<Vec<Bot>>>,
}

#[derive(Clone, Default)]
struct Inner {
    items: HashMap<SmolStr, Item>,
}

/// A client that can be composed from multiple subclients to interact with all of them as one.
///
/// # Bot IDs
///
/// [`BotId`]s are prefixed with the key used to insert the subclient. Take that into account when
/// calling [`send`](BotClient::send) or when reading the list of bots from [`bots`](BotClient::bots).
///
/// If you are just forwarding an id that came from [`bots`](BotClient::bots), to the [`send`](BotClient::send)
/// method, you don't need to worry.
///
/// If you are creating [`BotId`]s manually you can use [`RouterClient::prefix`] and [`RouterClient::unprefix`]
/// methods to help you.
///
/// # Cache
///
/// This router works by caching the results from calling [`bots`](BotClient::bots) on each sub-client.
/// The method will only be called for a specific sub-client if there is no cached result yet, or if
/// the cached result contains errors.
///
/// If a client is associated with a cached result, without errors, then its [`bots`](BotClient::bots) method
/// will not be called again.
///
/// If you need to refresh the list of bots, you will need to invalidate the cache explicitly.
#[derive(Clone, Default)]
pub struct RouterClient {
    inner: Arc<Mutex<Inner>>,
}

impl BotClient for RouterClient {
    fn bots(&self) -> BoxPlatformSendFuture<'static, ClientResult<Vec<Bot>>> {
        let me = self.clone();
        Box::pin(async move {
            me.cache_bots().await;

            let inner = me.inner.lock().unwrap();

            let mut value: Option<Vec<Bot>> = None;
            let mut errors = Vec::new();

            for (key, item) in inner.items.iter() {
                if let Some(result) = &item.bots_result {
                    errors.extend(result.errors().iter().cloned());

                    if value.is_none() {
                        value = Some(Vec::new());
                    }

                    if let Some(bots) = result.value() {
                        value.as_mut().unwrap().extend(bots.iter().map(|bot| Bot {
                            id: BotId::new(format!("{}/{}", key, bot.id.as_str())),
                            ..bot.clone()
                        }));
                    }
                }
            }

            (value, errors)
                .try_into()
                // The absence of both, value and errors, means the original list was empty.
                .unwrap_or_else(|_| ClientResult::new_ok(Vec::new()))
        })
    }

    fn send(
        &mut self,
        bot_id: &BotId,
        messages: &[Message],
        tools: &[Tool],
    ) -> BoxPlatformSendStream<'static, ClientResult<MessageContent>> {
        let bot_id = bot_id.clone();
        let messages = messages.to_vec();
        let tools = tools.to_vec();

        let me = self.clone();

        Box::pin(
            futures::stream::once(async move {
                let (key, id) = match bot_id.as_str().split_once('/') {
                    Some((k, i)) => (k, i),
                    None => {
                        let err = ClientError::new(
                            ClientErrorKind::Unknown,
                            format!("The bot id does not belong to a router: {:?}", bot_id),
                        );
                        let stream: BoxPlatformSendStream<_> =
                            Box::pin(futures::stream::once(async move { err.into() }));
                        return stream;
                    }
                };

                me.cache_bots().await;

                let mut client = match me.get_client(&key) {
                    Some(c) => c,
                    None => {
                        let err = ClientError::new(
                            ClientErrorKind::Unknown,
                            format!(
                                "This router has no client for the given bot id: {:?}",
                                bot_id
                            ),
                        );
                        let stream: BoxPlatformSendStream<_> =
                            Box::pin(futures::stream::once(async move { err.into() }));
                        return stream;
                    }
                };

                let bot_id = BotId::new(id);

                client.send(&bot_id, &messages, &tools)
            })
            .flatten(),
        )
    }

    fn clone_box(&self) -> Box<dyn BotClient> {
        Box::new(self.clone())
    }
}

impl RouterClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Invalidates the bots cache for all sub-clients.
    pub fn invalidate_all_bots_cache(&self) {
        let mut inner = self.inner.lock().unwrap();
        for item in inner.items.values_mut() {
            item.bots_result = None;
        }
    }

    /// Invalidates the bots cache for the client with the given key.
    pub fn invalidate_bots_cache(&self, key: impl AsRef<str>) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(item) = inner.items.get_mut(key.as_ref()) {
            item.bots_result = None;
        }
    }

    /// Inserts a client with the given key.
    pub fn insert_client(&self, key: impl AsRef<str>, client: Box<dyn BotClient>) {
        let mut inner = self.inner.lock().unwrap();
        inner.items.insert(
            key.as_ref().into(),
            Item {
                client,
                bots_result: None,
            },
        );
    }

    /// Removes a client by the key used to insert it.
    pub fn remove_client(&self, key: impl AsRef<str>) {
        let mut inner = self.inner.lock().unwrap();
        inner.items.remove(key.as_ref());
    }

    /// Gets a client by the key used to insert it.
    pub fn get_client(&self, key: impl AsRef<str>) -> Option<Box<dyn BotClient>> {
        let inner = self.inner.lock().unwrap();
        inner
            .items
            .get(key.as_ref())
            .map(|item| item.client.clone())
    }

    /// Caches the bots from all sub-clients that have not been cached yet, or that have errors.
    async fn cache_bots(&self) {
        // Collect entries quickly, before any async operation, to avoid retaining
        // the lock across await points.
        // These entries are either uncached entries, or entries that contain errors.
        let entries = self
            .inner
            .lock()
            .unwrap()
            .items
            .iter()
            .filter(|(_, item)| {
                item.bots_result
                    .as_ref()
                    .map(|r| r.has_errors())
                    .unwrap_or(true)
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>();

        if entries.is_empty() {
            return;
        }

        let bots = entries.iter().map(|(_, item)| item.client.bots());
        let bots = futures::future::join_all(bots).await;

        // Hold the lock to save the results.
        let mut inner = self.inner.lock().unwrap();
        for ((key, _), result) in entries.iter().zip(bots.into_iter()) {
            if let Some(entry) = inner.items.get_mut(key) {
                entry.bots_result = Some(result);
            }
        }
    }

    /// Prefixes a bot id with the given key.
    fn prefix(key: &str, bot_id: &BotId) -> BotId {
        BotId::new(format!("{}/{}", key, bot_id.as_str()))
    }

    /// Unprefixes a bot id, returning the key and the original bot id.
    fn unprefix(bot_id: &BotId) -> Option<(&str, BotId)> {
        let s = bot_id.as_str();
        let (key, id) = s.split_once('/')?;
        Some((key, BotId::new(id)))
    }
}
