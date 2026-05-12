use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::{CompletionRequest, ToolDefinition};
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

pub struct AzureProvider {
    client: Client,
    api_key: String,
    endpoint: String,
    deployment: String,
    api_version: String,
}

impl AzureProvider {
    pub fn from_env() -> ProviderResult<Self> {
        let api_key = std::env::var("AZURE_OPENAI_API_KEY")
            .map_err(|_| ProviderError::MissingApiKey)?;
        let endpoint = std::env::var("AZURE_OPENAI_ENDPOINT")
            .unwrap_or_else(|_| "https://your-resource.openai.azure.com".to_string());
        let deployment = std::env::var("AZURE_OPENAI_DEPLOYMENT")
            .unwrap_or_else(|_| "gpt-4o".to_string());
        let api_version = std::env::var("AZURE_OPENAI_API_VERSION")
            .unwrap_or_else(|_| "2024-02-15-preview".to_string());

        Ok(Self {
            client: Client::new(),
            api_key,
            endpoint,
            deployment,
            api_version,
        })
    }

    fn build_messages(&self, request: &CompletionRequest) -> Vec<AzureMessage> {
        request.messages.iter().map(|msg| AzureMessage { role: msg.role.clone(), content: msg.content.clone() }).collect()
    }

    fn build_request(&self, request: &CompletionRequest, stream: bool) -> AzureRequest {
        AzureRequest {
            messages: self.build_messages(request),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            tools: if request.tools.is_empty() { None } else { Some(request.tools.clone()) },
            stream: if stream { Some(true) } else { None },
        }
    }
}

#[derive(Serialize)]
struct AzureMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct AzureRequest {
    messages: Vec<AzureMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Deserialize)]
struct AzureResponse {
    choices: Vec<AzureChoice>,
    usage: AzureUsage,
}

#[derive(Deserialize)]
struct AzureChoice {
    message: AzureResponseMessage,
    finish_reason: String,
}

#[derive(Deserialize)]
struct AzureResponseMessage {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<AzureToolCallResponse>>,
}

#[derive(Deserialize)]
struct AzureToolCallResponse {
    id: String,
    #[serde(rename = "type")]
    tool_type: String,
    function: AzureFunctionResponse,
}

#[derive(Deserialize)]
struct AzureFunctionResponse {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct AzureUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

use lazy_static::lazy_static;

lazy_static! {
    static ref AZURE_MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("gpt-4o")),
            name: Some("GPT-4o (Azure)".to_string()),
            family: Some("gpt-4".to_string()),
            release_date: None,
            attachment: None,
            reasoning: None,
            temperature: None,
            tool_call: None,
            interleaved: None,
            cost: None,
            limit: None,
            modalities: None,
            experimental: None,
            status: None,
            provider: None,
            options: None,
            headers: None,
            variants: None,
        },
    ];
}

#[async_trait::async_trait]
impl Provider for AzureProvider {
    fn name(&self) -> &str {
        "azure"
    }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let url = format!(
            "{}openai/deployments/{}/chat/completions?api-version={}",
            self.endpoint, self.deployment, self.api_version
        );

        let azure_req = self.build_request(&request, false);

        let response = self.client
            .post(&url)
            .header("api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&azure_req)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await?;
            return Err(ProviderError::api(status, body));
        }

        let azure_resp: AzureResponse = response.json().await?;

        let content = azure_resp.choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        let tool_calls: Vec<ToolCall> = azure_resp.choices
            .first()
            .and_then(|c| c.message.tool_calls.as_ref())
            .map(|tc| tc.iter().map(|t| ToolCall {
                id: t.id.clone(),
                name: t.function.name.clone(),
                arguments: t.function.arguments.clone(),
            }).collect())
            .unwrap_or_default();

        Ok(CompletionResponse {
            content,
            tool_calls,
            stop_reason: Some(azure_resp.choices.first().map(|c| c.finish_reason.clone()).unwrap_or_default()),
            usage: TokenUsage {
                input: azure_resp.usage.prompt_tokens,
                output: azure_resp.usage.completion_tokens,
                cache_read: None,
                cache_write: None,
            },
            model: self.deployment.clone(),
        })
    }

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
        let azure_req = self.build_request(&request, true);
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let endpoint = self.endpoint.clone();
        let deployment = self.deployment.clone();
        let api_version = self.api_version.clone();

        let stream = async_stream::try_stream! {
            let url = format!(
                "{}openai/deployments/{}/chat/completions?api-version={}",
                endpoint, deployment, api_version
            );

            let response = client
                .post(&url)
                .header("api-key", api_key)
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&azure_req)
                .send()
                .await?;

            let response = response.error_for_status()
                .map_err(|e| ProviderError::api(e.status().map(|s| s.as_u16()).unwrap_or(0), e.to_string()))?;

            let mut stream_reader = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = stream_reader.next().await.transpose()? {
                let text = String::from_utf8_lossy(&chunk);
                buffer.push_str(&text);

                let lines: Vec<String> = buffer.split('\n').map(String::from).collect();

                if lines.len() <= 1 {
                    if let Some(last) = lines.last() {
                        buffer = last.clone();
                    }
                    continue;
                }

                buffer = lines.last().cloned().unwrap_or_default();

                for line in &lines[..lines.len() - 1] {
                    let line = line.trim();
                    if line.is_empty() || !line.starts_with("data: ") {
                        continue;
                    }

                    let data = &line[6..];
                    if data == "[DONE]" {
                        yield StreamEvent::message_stop(
                            "stop".to_string(),
                            TokenUsage { input: 0, output: 0, cache_read: None, cache_write: None },
                        );
                        continue;
                    }

                    if let Ok(event) = Self::parse_sse_event(data) {
                        yield event;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    fn models(&self) -> &[ModelInfo] {
        &AZURE_MODELS
    }

    fn default_model(&self) -> Option<&ModelInfo> {
        AZURE_MODELS.first()
    }
}

impl AzureProvider {
    fn parse_sse_event(data: &str) -> Result<StreamEvent, serde_json::Error> {
        #[derive(Debug, Deserialize)]
        struct AzureSseResponse {
            id: Option<String>,
            choices: Vec<AzureSseChoice>,
        }

        #[derive(Debug, Deserialize)]
        struct AzureSseChoice {
            index: u32,
            delta: AzureSseDelta,
            finish_reason: Option<String>,
        }

        #[derive(Debug, Deserialize)]
        struct AzureSseDelta {
            role: Option<String>,
            content: Option<String>,
            tool_calls: Option<Vec<AzureSseToolCall>>,
        }

        #[derive(Debug, Deserialize)]
        struct AzureSseToolCall {
            index: u32,
            id: Option<String>,
            function: Option<AzureSseFunction>,
        }

        #[derive(Debug, Deserialize)]
        struct AzureSseFunction {
            name: Option<String>,
            arguments: Option<String>,
        }

        let response: AzureSseResponse = serde_json::from_str(data)?;

        let choice = response.choices.first();
        let delta_content = choice.and_then(|c| c.delta.content.clone());
        let finish_reason = choice.and_then(|c| c.finish_reason.clone());

        let tool_call = choice.and_then(|c| c.delta.tool_calls.as_ref())
            .and_then(|tc| tc.first())
            .and_then(|t| {
                Some(ToolCall {
                    id: t.id.clone()?,
                    name: t.function.as_ref()?.name.clone()?,
                    arguments: t.function.as_ref()?.arguments.clone()?,
                })
            });

        Ok(StreamEvent {
            event_type: if finish_reason.is_some() { "message_stop" } else { "content_block_delta" }.to_string(),
            delta: delta_content,
            tool_call,
            stop_reason: finish_reason,
            usage: None,
        })
    }
}