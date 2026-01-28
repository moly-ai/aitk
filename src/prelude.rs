//! Re-exports common abstractions that are likely to be used.

// The most important module in this crate.
pub use crate::protocol::*;

// These are the clients that are most commonly used.
#[cfg(feature = "api-clients")]
pub use crate::clients::openai::OpenAiClient;
pub use crate::clients::router::RouterClient;

// These other clients are less commonly used.
#[cfg(feature = "api-clients")]
pub use crate::clients::openai_image::OpenAiImageClient;
#[cfg(feature = "realtime-clients")]
pub use crate::clients::openai_realtime::OpenAiRealtimeClient;
#[cfg(feature = "api-clients")]
pub use crate::clients::openai_stt::OpenAiSttClient;
pub use crate::clients::{map::MapClient, tester::TesterClient};

// If we re-export clients, then we may also re-export tools.
#[cfg(feature = "mcp")]
pub use crate::mcp::mcp_manager::{McpManagerClient, McpTransport};

// Only used by users that want the built-in chat business logic. But this is expected.
pub use crate::controllers::chat::*;

// Common mutation types used by controllers.
pub use crate::utils::vec::{IndexSet, VecEffect, VecMutation};
