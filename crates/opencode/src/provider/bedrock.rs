// Bedrock provider stub.
// The original implementation depended on `aws-config` and `aws-sdk-bedrockruntime`,
// which are not currently in Cargo.toml. Keep a compile-clean stub so the rest of
// the workspace builds; wiring real Bedrock support is a follow-up.

use async_trait::async_trait;
use lazy_static::lazy_static;

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::CompletionResponse;
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

lazy_static! {
    static ref MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("anthropic.claude-3-5-sonnet-20240620-v1:0")),
            name: Some("Claude 3.5 Sonnet (Bedrock)".to_string()),
            family: Some("claude".to_string()),
            reasoning: Some(false),
            tool_call: Some(true),
            attachment: Some(true),
            temperature: Some(true),
            interleaved: None,
            cost: None,
            limit: None,
            modalities: None,
            experimental: None,
            release_date: None,
            status: Some("active".to_string()),
            provider: None,
            options: None,
            headers: None,
            variants: None,
        },
    ];
}

pub struct BedrockProvider;

impl BedrockProvider {
    pub fn new() -> Self { Self }
    pub fn from_env() -> ProviderResult<Self> { Ok(Self) }
}

#[async_trait]
impl Provider for BedrockProvider {
    fn name(&self) -> &str { "bedrock" }
    fn default_model(&self) -> Option<&ModelInfo> { MODELS.first() }
    fn models(&self) -> &[ModelInfo] { &MODELS }

    async fn complete(&self, _request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        Err(ProviderError::api(501, "bedrock provider not yet implemented"))
    }

    fn stream(&self, _request: CompletionRequest) -> ProviderResult<EventStream> {
        Err(ProviderError::stream("bedrock provider not yet implemented"))
    }
}
