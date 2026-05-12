use std::str::FromStr;

use derive_more::Display;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ulid::Ulid;

/// Permission identifier (ULID-based)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display)]
#[display("{inner}")]
pub struct PermissionID {
    inner: Ulid,
}

impl PermissionID {
    /// Generate a new permission ID
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

impl Default for PermissionID {
    fn default() -> Self {
        Self::new()
    }
}

impl Serialize for PermissionID {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.inner.to_string())
    }
}

impl<'de> Deserialize<'de> for PermissionID {
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

impl FromStr for PermissionID {
    type Err = ulid::DecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl From<Ulid> for PermissionID {
    fn from(inner: Ulid) -> Self {
        Self { inner }
    }
}

impl From<PermissionID> for Ulid {
    fn from(id: PermissionID) -> Self {
        id.inner
    }
}
