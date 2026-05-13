pub mod assistant;
pub mod error;
pub mod format;
pub mod part;
pub mod tool_state;
pub mod user;

pub use assistant::*;
pub use error::*;
pub use format::*;
pub use part::*;
pub use tool_state::*;
pub use user::*;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    User(UserMessage),
    Assistant(AssistantMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WithParts {
    pub info: Message,
    pub parts: Vec<Part>,
}
