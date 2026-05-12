use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Provider authentication error
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAuthError {
    pub provider_id: String,
    pub message: String,
}

/// Unknown error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnknownError {
    pub message: String,
}

/// Message output length exceeded error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageOutputLengthError {}

/// Message aborted error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageAbortedError {
    pub message: String,
}

/// Structured output error
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredOutputError {
    pub message: String,
    pub retries: u32,
}

/// Context overflow error
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextOverflowError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_body: Option<String>,
}

/// API error
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct APIError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u32>,
    pub is_retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_headers: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

/// Union enum for all assistant errors, tagged by "name"
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "name", rename_all = "PascalCase")]
pub enum AssistantError {
    ProviderAuthError(ProviderAuthError),
    UnknownError(UnknownError),
    MessageOutputLengthError(MessageOutputLengthError),
    MessageAbortedError(MessageAbortedError),
    StructuredOutputError(StructuredOutputError),
    ContextOverflowError(ContextOverflowError),
    APIError(APIError),
}
