//! Re-exports common abstractions that are likely to be used.

// The most important module in this crate.
pub use crate::protocol::*;

// These are the clients that are most commonly used.
pub use crate::clients::multi::MultiClient;
#[cfg(all(feature = "json", feature = "http"))]
pub use crate::clients::openai::OpenAiClient;

// These other clients are less commonly used.
pub use crate::clients::{map::MapClient, tester::TesterClient};
#[cfg(all(feature = "json", feature = "http"))]
pub use crate::clients::{openai_image::OpenAiImageClient, openai_realtime::OpenAiRealtimeClient};

// If we re-export clients, then we may also re-export tools.
#[cfg(all(not(target_arch = "wasm32"), feature = "mcp"))]
pub use crate::mcp::mcp_manager::{McpManagerClient, McpTransport};

// Only used by users that want the built-in chat business logic. But this is expected.
pub use crate::controllers::chat::*;

// Common mutation types used by controllers.
pub use crate::utils::vec::{IndexSet, VecEffect, VecMutation};
