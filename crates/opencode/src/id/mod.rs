mod message;
mod part;
mod project;
/// Type-safe ULID-based identifiers for OpenCode entities.
///
/// All ID types are newtype wrappers around `ulid::Ulid`, providing:
/// - Time-sortable identifiers (26-character Crockford Base32)
/// - Serialization/Deserialization via serde
/// - Display and FromStr implementations
/// - Type-safe boundaries between different entity identifiers
mod session;
mod workspace;

pub use message::MessageID;
pub use part::PartID;
pub use project::ProjectID;
pub use session::SessionID;
pub use workspace::WorkspaceID;

/// Common trait for all identifier types.
pub trait Identifier:
    std::fmt::Display + std::str::FromStr + Clone + PartialEq + Eq + std::hash::Hash
{
    /// Generate a identifier.
    fn new() -> Self;

    /// Get the inner ULID as a string.
    fn to_string(&self) -> String;

    /// Parse an identifier from a string.
    fn from_str(s: &str) -> Result<Self, Self::Err>;
}