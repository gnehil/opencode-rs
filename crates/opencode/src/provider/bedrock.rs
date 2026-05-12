use async_trait::async_trait;
use reqwest::Client;
use lazy_static::lazy_static;
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_bedrockruntime::Client as BedrockClient;

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

lazy_static! {
    static ref MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
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
        ModelInfo {
            id: Some(ModelID::new("amazon.nova-pro-v1:0")),
            name: Some("Amazon Nova Pro".to_string()),
            family: Some("nova".to_string()),
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

pub struct BedrockProvider {
    client: Option<BedrockClient>,
    region: String,
}

impl BedrockProvider {
    pub fn new(region: Option<String>) -> Self {
        Self {
            client: None,
            region: region.unwrap_or_else(|| "us-east-1".to_string()),
        }
    }

    pub async fn from_env() -> ProviderResult<Self> {
        let region = std::env::var("AWS_REGION").ok();
        Ok(Self::new(region))
    }

    async fn init_client(&self) -> BedrockClient {
        let region_provider = RegionProviderChain::first_match(self.region.clone())
            .or_default_provider();
        let config = aws_config::from_env().region(region_provider).load().await;
        BedrockClient::new(&config)
    }
}

impl Provider for BedrockProvider {
    fn name(&self) -> &str { "amazon-bedrock" }
    fn default_model(&self) -> Option<&ModelInfo> { MODELS.first() }
    fn models(&self) -> &[ModelInfo] { &MODELS }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        Ok(CompletionResponse {
            content: "Bedrock integration requires AWS SDK setup. Configure AWS credentials and region.".to_string(),
            tool_calls: vec![],
            usage: TokenUsage { input: 0, output: 0, cache_read: None, cache_write: None },
            stop_reason: "stop".to_string(),
        })
    }

    async fn stream(&self, _request: CompletionRequest) -> ProviderResult<EventStream> { Err(ProviderError::StreamNotSupported) }
}