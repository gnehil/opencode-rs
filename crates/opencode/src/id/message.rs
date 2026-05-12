use std::str::FromStr;

use derive_more::Display;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ulid::Ulid;

/// Message identifier (ULID-based)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display)]
#[display("{inner}")]
pub struct MessageID {
    inner: Ulid,
}

impl MessageID {
    /// Generate a new message ID
    pub fn new() -> Self {
        Self { inner: Ulid::new() }
    }

    /// Get the inner ULID
    pub fn ulid(&self) -> Ulid {
        self.inner
    }

    /// Parse from string
    pub fn parse(s: &str) -> Result<Self, ulid::DecodeError> {
        Ulid::from_str(s).map(|inner| Self { inner })
    }
}

impl Default for MessageID {
    fn default() -> Self {
        Self::new()
    }
}

impl Serialize for MessageID {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.inner.to_string())
    }
}

impl<'de> Deserialize<'de> for MessageID {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ulid::from_str(&s)
            .map_err(serde::de::Error::custom)
            .map(|inner| Self { inner })
    }
}

impl FromStr for MessageID {
    type Err = ulid::DecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl From<Ulid> for MessageID {
    fn from(inner: Ulid) -> Self {
        Self { inner }
    }
}

impl From<MessageID> for Ulid {
    fn from(id: MessageID) -> Self {
        id.inner
    }
}
