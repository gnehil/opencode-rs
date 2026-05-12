use std::pin::Pin;

use futures::Stream;

use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent};
use super::model::ModelInfo;

pub type ProviderResult<T> = Result<T, ProviderError>;
pub type EventStream = Pin<Box<dyn Stream<Item = ProviderResult<StreamEvent>> + Send + 'static>>;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error: {status} - {message}")]
    Api { status: u16, message: String },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Stream error: {0}")]
    StreamError(String),

    #[error("Provider not configured: missing API key")]
    MissingApiKey,

    #[error("Model not found: {0}")]
    ModelNotFound(String),
}

impl ProviderError {
    pub fn api(status: u16, message: impl Into<String>) -> Self {
        Self::Api { status, message: message.into() }
    }

    pub fn stream(message: impl Into<String>) -> Self {
        Self::StreamError(message.into())
    }
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse>;

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream>;

    fn models(&self) -> &[ModelInfo];

    fn default_model(&self) -> Option<&ModelInfo>;
}