//! Provider and Model identifier types.
//!
//! These are branded string types (not ULIDs) that provide type-safe
//! boundaries between provider and model identifiers.

use std::str::FromStr;

use derive_more::Display;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::{Display as StrumDisplay, EnumString};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Display)]
#[display("{inner}")]
pub struct ProviderID {
    inner: String,
}

impl ProviderID {
    pub const OPENCODE: &'static str = "opencode";
    pub const ANTHROPIC: &'static str = "anthropic";
    pub const OPENAI: &'static str = "openai";
    pub const GOOGLE: &'static str = "google";
    pub const GOOGLE_VERTEX: &'static str = "google-vertex";
    pub const GITHUB_COPILOT: &'static str = "github-copilot";
    pub const AMAZON_BEDROCK: &'static str = "amazon-bedrock";
    pub const AZURE: &'static str = "azure";
    pub const OPENROUTER: &'static str = "openrouter";
    pub const MISTRAL: &'static str = "mistral";
    pub const GITLAB: &'static str = "gitlab";

    /// Create a new ProviderID from a string.
    pub fn new(s: impl Into<String>) -> Self {
        Self { inner: s.into() }
    }

    /// Get the inner string reference.
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Convert into the inner String.
    pub fn into_inner(self) -> String {
        self.inner
    }

    /// Well-known provider ID instances.
    pub fn opencode() -> Self {
        Self::new(Self::OPENCODE)
    }

    pub fn anthropic() -> Self {
        Self::new(Self::ANTHROPIC)
    }

    pub fn openai() -> Self {
        Self::new(Self::OPENAI)
    }

    pub fn google() -> Self {
        Self::new(Self::GOOGLE)
    }

    pub fn google_vertex() -> Self {
        Self::new(Self::GOOGLE_VERTEX)
    }

    pub fn github_copilot() -> Self {
        Self::new(Self::GITHUB_COPILOT)
    }

    pub fn amazon_bedrock() -> Self {
        Self::new(Self::AMAZON_BEDROCK)
    }

    pub fn azure() -> Self {
        Self::new(Self::AZURE)
    }

    pub fn openrouter() -> Self {
        Self::new(Self::OPENROUTER)
    }

    pub fn mistral() -> Self {
        Self::new(Self::MISTRAL)
    }

    pub fn gitlab() -> Self {
        Self::new(Self::GITLAB)
    }
}

impl Serialize for ProviderID {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.inner)
    }
}

impl<'de> Deserialize<'de> for ProviderID {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Self { inner: s })
    }
}

impl FromStr for ProviderID {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

impl From<String> for ProviderID {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for ProviderID {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl AsRef<str> for ProviderID {
    fn as_ref(&self) -> &str {
        &self.inner
    }
}

// ---------------------------------------------------------------------------
// ModelID
// ---------------------------------------------------------------------------

/// Branded string type for model identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Display)]
#[display("{inner}")]
pub struct ModelID {
    inner: String,
}

impl ModelID {
    /// Create a new ModelID from a string.
    pub fn new(s: impl Into<String>) -> Self {
        Self { inner: s.into() }
    }

    /// Get the inner string reference.
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Convert into the inner String.
    pub fn into_inner(self) -> String {
        self.inner
    }
}

impl Serialize for ModelID {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.inner)
    }
}

impl<'de> Deserialize<'de> for ModelID {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Self { inner: s })
    }
}

impl FromStr for ModelID {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

impl From<String> for ModelID {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for ModelID {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl AsRef<str> for ModelID {
    fn as_ref(&self) -> &str {
        &self.inner
    }
}

// ---------------------------------------------------------------------------
// Provider (enum of well-known providers)
// ---------------------------------------------------------------------------

/// Enum of well-known provider names for pattern matching and display.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumString, StrumDisplay,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    Opencode,
    Anthropic,
    Openai,
    Google,
    GoogleVertex,
    GithubCopilot,
    AmazonBedrock,
    Azure,
    Openrouter,
    Mistral,
    Gitlab,
}

impl Provider {
    pub fn to_id(&self) -> ProviderID {
        ProviderID::new(self.to_string())
    }
}

impl From<Provider> for ProviderID {
    fn from(provider: Provider) -> Self {
        provider.to_id()
    }
}
