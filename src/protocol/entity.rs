use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::collections::HashSet;
use std::fmt;

/// The picture/avatar of an entity that may be represented/encoded in different ways.
// TODO: Consider Arc<str> where applicable.
#[derive(Clone, Debug, PartialEq)]
pub enum EntityAvatar {
    /// Normally, one or two graphemes representing the entity.
    Text(String),
    /// An image located at the given path/URL.
    Image(String),
}

impl EntityAvatar {
    /// Utility to construct a [`Picture::Text`] from a single grapheme.
    ///
    /// Extracted using unicode segmentation.
    pub fn from_first_grapheme(text: &str) -> Option<Self> {
        use unicode_segmentation::UnicodeSegmentation;
        text.graphemes(true)
            .next()
            .map(|g| g.to_string())
            .map(EntityAvatar::Text)
    }
}

/// Indentify the entities that are recognized by this crate, mainly in a chat.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum EntityId {
    /// Represents the user operating this app.
    User,

    /// Represents the `system`/`developer` expected by many LLMs in the chat
    /// context to customize the chat experience and behavior.
    System,

    /// Represents a bot, which is an automated assistant of any kind (model, agent, etc).
    Bot(BotId),

    /// Represents tool execution results and tool-related system messages.
    /// Maps to the "tool" role in LLM APIs.
    Tool,

    /// This app itself. Normally appears when app specific information must be displayed
    /// (like inline errors).
    ///
    /// It's not supposed to be sent as part of a conversation to bots.
    #[default]
    App,
}

/// Represents the capabilities of a bot
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BotCapability {
    /// Bot supports text input.
    TextInput,
    /// Bot can answer with text output.
    TextOutput,
    /// Bot supports starting a realtime audio call for conversation.
    Realtime,
    /// Bot accepts attachments as input.
    AttachmentInput,
    /// Bot can produce attachments as output.
    AttachmentOutput,
    /// Bot supports function calling (tools).
    FunctionCalling,
}

/// Set of capabilities that a bot supports
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BotCapabilities {
    // TODO: Heap allocations, hashing, indirections, etc to store a known-size
    // bunch of bools is probably an overkill. Consider switching the internal
    // representation to something like the `enumset` crate, or the `bitflags` crate.
    capabilities: HashSet<BotCapability>,
}

impl BotCapabilities {
    pub fn new() -> Self {
        Self {
            capabilities: HashSet::new(),
        }
    }

    pub fn all() -> Self {
        let mut capabilities = HashSet::new();
        capabilities.insert(BotCapability::TextInput);
        capabilities.insert(BotCapability::TextOutput);
        capabilities.insert(BotCapability::Realtime);
        capabilities.insert(BotCapability::AttachmentInput);
        capabilities.insert(BotCapability::AttachmentOutput);
        capabilities.insert(BotCapability::FunctionCalling);
        Self { capabilities }
    }

    pub fn with_capability(self, capability: BotCapability) -> Self {
        self.with_capabilities([capability])
    }

    pub fn add_capability(&mut self, capability: BotCapability) {
        self.add_capabilities([capability]);
    }

    pub fn with_capabilities(
        mut self,
        capabilities: impl IntoIterator<Item = BotCapability>,
    ) -> Self {
        self.add_capabilities(capabilities);
        self
    }

    pub fn add_capabilities(&mut self, capabilities: impl IntoIterator<Item = BotCapability>) {
        for capability in capabilities {
            self.capabilities.insert(capability);
        }
    }

    pub fn has_capability(&self, capability: &BotCapability) -> bool {
        self.capabilities.contains(capability)
    }

    pub fn iter(&self) -> impl Iterator<Item = &BotCapability> {
        self.capabilities.iter()
    }
}

/// Represents a bot, which is an automated assistant of any kind (model, agent, etc).
///
/// # WARNING
///
/// This is an "ideal" representation. However, OpenAI-compatible APIs will only
/// give us the model id (though the `/models` endpoint) and nothing else. Therefore,
/// for most client implementations hitting such APIs, the name and the avatar may be
/// set from the id, and capabilities will be a best-effort guess or a fixed set based
/// on the client itself.
///
/// For example, the [`crate::clients::openai::OpenAiClient`] will simply list all
/// models available at `/models`, with a [`BotCapability::TextOutput`] as this client
/// is intended for text-based conversations. However, realtime and image models will also
/// be there with that capability incorrectly set.
///
/// Depending on your use case, it recommended to either:
/// - Ignore the capabilities field for [`Bot`]s coming from such clients.
/// - Override them if you are working with concrete models you know the capabilities of.
/// - Try to filter models that should not be listed by the client in the first place (e.g.,
///   image and realtime models in a text-only client).
#[derive(Clone, Debug, PartialEq)]
pub struct Bot {
    pub id: BotId,
    pub name: String,
    pub avatar: EntityAvatar,
    pub capabilities: BotCapabilities,
}

/// Identifies any kind of bot, local or remote, model or agent, whatever.
///
/// Normally, this is just the model name or id as known by the provider.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default, Serialize)]
pub struct BotId(SmolStr);

impl<'de> Deserialize<'de> for BotId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// V1 compatibility parser.
        ///
        /// The id is encoded as: <id_len>;<id>@<provider>.
        /// `@` is simply a semantic separator, meaning (literally) "at".
        /// The length is what is actually used for separating components allowing
        /// these to include `@` characters.
        ///
        /// Example: `9;qwen:0.5b@http://localhost:11434/v1`
        fn v1(raw: &SmolStr) -> Option<SmolStr> {
            let (id_length, raw) = raw.split_once(';')?;
            let id_length = id_length.parse::<usize>().ok()?;
            let id = &raw[..id_length];
            // + 1 skips the semantic `@` separator
            let _provider = &raw[id_length + 1..];
            Some(id.into())
        }

        // Read the raw payload.
        let raw = SmolStr::deserialize(deserializer)?;

        // Try to parse as v1 first.
        if let Some(s) = v1(&raw) {
            return Ok(BotId(s));
        }

        // Raw should be the current representation.
        Ok(BotId(raw))
    }
}

impl BotId {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Creates a new bot id from a provider specific id.
    pub fn new(id: impl AsRef<str>) -> Self {
        BotId(id.as_ref().into())
    }

    /// The id of the bot as it is known by its provider. The "model name".
    ///
    /// This should be equivalent to [`BotId::as_str`].
    pub fn id(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for BotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
