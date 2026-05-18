use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use super::session_handlers::AppState;
use crate::auth::{AuthInfo, AuthStore};
use crate::config::{Config, ModelConfig, ProviderConfigEntry};
use crate::provider::{
    AlibabaProvider, AnthropicProvider, BedrockProvider, CerebrasProvider, CohereProvider,
    DeepInfraProvider, DeepSeekProvider, FireworksProvider, GitHubCopilotProvider, GitLabProvider,
    GoogleProvider, GroqProvider, LMStudioProvider, MistralProvider, ModelInfo, OllamaProvider,
    OpenAIProvider, OpenRouterProvider, PerplexityProvider, Provider, TogetherAIProvider,
    VeniceProvider, VercelProvider, VertexProvider, XAIProvider,
};

pub async fn get_config(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let value = state
        .config
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .unwrap_or_else(|| json!({}));
    Ok(Json(value))
}

pub async fn update_config(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let old_value = state
        .config
        .as_ref()
        .and_then(|config| serde_json::to_value(config).ok())
        .unwrap_or(serde_json::Value::Null);
    if let Err(error) = state
        .plugin_manager
        .trigger_config_change(crate::plugin::ConfigChangeInput {
            config_type: "project".to_string(),
            old_value,
            new_value: body.clone(),
        })
        .await
    {
        tracing::warn!("plugin config hook failed: {}", error);
    }
    Ok(Json(body))
}

pub async fn list_providers(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let inventory = provider_inventory(&state).await?;
    Ok(Json(json!({
        "all": inventory.all.values().cloned().collect::<Vec<_>>(),
        "default": default_model_ids(&inventory.all),
        "connected": inventory.connected.into_iter().collect::<Vec<_>>(),
    })))
}

pub async fn config_providers(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let inventory = provider_inventory(&state).await?;
    let providers = inventory
        .connected
        .iter()
        .filter_map(|id| inventory.all.get(id).cloned())
        .collect::<Vec<_>>();
    let provider_map = providers
        .iter()
        .filter_map(|provider| {
            provider
                .get("id")
                .and_then(Value::as_str)
                .map(|id| (id.to_string(), provider.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    Ok(Json(json!({
        "providers": providers,
        "default": default_model_ids(&provider_map),
    })))
}

pub async fn provider_auth_methods(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(provider_auth_method_map()))
}

#[derive(Deserialize)]
pub struct ProviderAuthorizeBody {
    method: usize,
    inputs: Option<HashMap<String, String>>,
}

pub async fn provider_oauth_authorize(
    Path(provider_id): Path<String>,
    Json(body): Json<ProviderAuthorizeBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let methods = provider_auth_method_map();
    let method = methods
        .get(&provider_id)
        .and_then(Value::as_array)
        .and_then(|items| items.get(body.method))
        .ok_or(StatusCode::BAD_REQUEST)?;

    if method.get("type").and_then(Value::as_str) == Some("api") {
        return Ok(Json(Value::Null));
    }

    let _ = body.inputs;
    Err(StatusCode::BAD_REQUEST)
}

#[derive(Deserialize)]
pub struct ProviderCallbackBody {
    method: usize,
    code: Option<String>,
}

pub async fn provider_oauth_callback(
    Path(_provider_id): Path<String>,
    Json(body): Json<ProviderCallbackBody>,
) -> Result<Json<bool>, StatusCode> {
    let _ = (body.method, body.code);
    Err(StatusCode::BAD_REQUEST)
}

struct ProviderInventory {
    all: BTreeMap<String, Value>,
    connected: BTreeSet<String>,
}

async fn provider_inventory(state: &AppState) -> Result<ProviderInventory, StatusCode> {
    let mut all = builtin_provider_infos();
    let mut connected = BTreeSet::new();

    apply_configured_providers(&mut all, &mut connected, state.config.as_ref());
    apply_provider_filter(&mut all, &mut connected, state.config.as_ref());
    apply_environment_connections(&mut all, &mut connected);
    apply_auth_connections(&mut all, &mut connected, state).await?;
    apply_state_provider(&mut all, &mut connected, state);

    Ok(ProviderInventory { all, connected })
}

fn builtin_provider_infos() -> BTreeMap<String, Value> {
    let mut providers = BTreeMap::new();

    insert_provider(
        &mut providers,
        "alibaba",
        "Alibaba",
        &["ALIBABA_API_KEY", "DASHSCOPE_API_KEY"],
        AlibabaProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "anthropic",
        "Anthropic",
        &["ANTHROPIC_API_KEY"],
        AnthropicProvider::new("test".to_string(), None).models(),
    );
    insert_provider(
        &mut providers,
        "amazon-bedrock",
        "Amazon Bedrock",
        &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"],
        BedrockProvider::new(
            "test".to_string(),
            "test".to_string(),
            None,
            "us-east-1".to_string(),
        )
        .models(),
    );
    insert_provider(
        &mut providers,
        "azure",
        "Azure OpenAI",
        &["AZURE_OPENAI_API_KEY", "AZURE_OPENAI_ENDPOINT"],
        &[ModelInfo {
            id: Some("gpt-4o".into()),
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
        }],
    );
    insert_provider(
        &mut providers,
        "cerebras",
        "Cerebras",
        &["CEREBRAS_API_KEY"],
        CerebrasProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "cohere",
        "Cohere",
        &["COHERE_API_KEY"],
        CohereProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "deepinfra",
        "DeepInfra",
        &["DEEPINFRA_API_KEY"],
        DeepInfraProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "deepseek",
        "DeepSeek",
        &["DEEPSEEK_API_KEY"],
        DeepSeekProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "fireworks",
        "Fireworks",
        &["FIREWORKS_API_KEY"],
        FireworksProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "github-copilot",
        "GitHub Copilot",
        &["GITHUB_COPILOT_TOKEN"],
        GitHubCopilotProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "gitlab",
        "GitLab",
        &["GITLAB_TOKEN"],
        GitLabProvider::new(None, "test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "google",
        "Google",
        &["GOOGLE_GENERATIVE_AI_API_KEY", "GEMINI_API_KEY"],
        GoogleProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "google-vertex",
        "Google Vertex",
        &["GOOGLE_ACCESS_TOKEN", "GOOGLE_PROJECT_ID", "GCP_PROJECT_ID"],
        VertexProvider::new(
            "project".to_string(),
            "us-central1".to_string(),
            "token".to_string(),
        )
        .models(),
    );
    insert_provider(
        &mut providers,
        "groq",
        "Groq",
        &["GROQ_API_KEY"],
        GroqProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "lmstudio",
        "LM Studio",
        &[],
        LMStudioProvider::new(None).models(),
    );
    insert_provider(
        &mut providers,
        "mistral",
        "Mistral",
        &["MISTRAL_API_KEY"],
        MistralProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "ollama",
        "Ollama",
        &[],
        OllamaProvider::new(None).models(),
    );
    insert_provider(
        &mut providers,
        "openai",
        "OpenAI",
        &["OPENAI_API_KEY"],
        OpenAIProvider::new("test".to_string(), None).models(),
    );
    insert_provider(
        &mut providers,
        "openrouter",
        "OpenRouter",
        &["OPENROUTER_API_KEY"],
        OpenRouterProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "perplexity",
        "Perplexity",
        &["PERPLEXITY_API_KEY"],
        PerplexityProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "togetherai",
        "Together AI",
        &["TOGETHER_API_KEY", "TOGETHERAI_API_KEY"],
        TogetherAIProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "venice",
        "Venice",
        &["VENICE_API_KEY"],
        VeniceProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "vercel",
        "Vercel",
        &["VERCEL_AI_GATEWAY_API_KEY"],
        VercelProvider::new("test".to_string()).models(),
    );
    insert_provider(
        &mut providers,
        "xai",
        "xAI",
        &["XAI_API_KEY"],
        XAIProvider::new("test".to_string()).models(),
    );

    providers
}

fn insert_provider(
    providers: &mut BTreeMap<String, Value>,
    id: &str,
    name: &str,
    env: &[&str],
    models: &[ModelInfo],
) {
    providers.insert(
        id.to_string(),
        json!({
            "id": id,
            "name": name,
            "source": "custom",
            "env": env,
            "options": {},
            "models": model_record(id, models),
        }),
    );
}

fn model_record(provider_id: &str, models: &[ModelInfo]) -> Value {
    let mut record = serde_json::Map::new();
    for (index, model) in models.iter().enumerate() {
        let id = model
            .id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("model-{index}"));
        record.insert(id.clone(), public_model_from_info(provider_id, &id, model));
    }
    Value::Object(record)
}

fn public_model_from_info(provider_id: &str, id: &str, model: &ModelInfo) -> Value {
    let mut value = serde_json::to_value(model).unwrap_or_else(|_| json!({}));
    let obj = value.as_object_mut().expect("model serializes as object");
    obj.insert("id".to_string(), json!(id));
    obj.entry("name".to_string()).or_insert_with(|| json!(id));
    obj.insert("providerID".to_string(), json!(provider_id));
    obj.insert(
        "api".to_string(),
        json!({
            "id": id,
            "npm": provider_npm(provider_id),
            "url": "",
        }),
    );
    obj.insert(
        "capabilities".to_string(),
        json!({
            "temperature": model.temperature.unwrap_or(false),
            "reasoning": model.reasoning.unwrap_or(false),
            "attachment": model.attachment.unwrap_or(false),
            "toolcall": model.tool_call.unwrap_or(true),
            "input": {
                "text": true,
                "audio": false,
                "image": model.attachment.unwrap_or(false),
                "video": false,
                "pdf": false,
            },
            "output": {
                "text": true,
                "audio": false,
                "image": false,
                "video": false,
                "pdf": false,
            },
            "interleaved": model.interleaved.is_some(),
        }),
    );
    obj.insert("cost".to_string(), public_model_info_cost(model));
    obj.insert("limit".to_string(), public_model_info_limit(model));
    value
}

fn public_model_info_cost(model: &ModelInfo) -> Value {
    let Some(cost) = model.cost.as_ref() else {
        return json!({ "input": 0, "output": 0, "cache": { "read": 0, "write": 0 } });
    };
    json!({
        "input": cost.input,
        "output": cost.output,
        "cache": {
            "read": cost.cache_read.unwrap_or(0.0),
            "write": cost.cache_write.unwrap_or(0.0),
        },
    })
}

fn public_model_info_limit(model: &ModelInfo) -> Value {
    let Some(limit) = model.limit.as_ref() else {
        return json!({ "context": 0, "output": 0 });
    };
    json!({
        "context": limit.context,
        "input": limit.input,
        "output": limit.output,
    })
}

fn public_model_from_config(provider_id: &str, id: &str, model: &ModelConfig) -> Value {
    let mut value = serde_json::to_value(model).unwrap_or_else(|_| json!({}));
    let api_id = model.id.as_deref().unwrap_or(id);
    let name = model.name.as_deref().unwrap_or(id);
    let obj = value.as_object_mut().expect("model serializes as object");
    obj.insert("id".to_string(), json!(id));
    obj.insert("name".to_string(), json!(name));
    obj.insert("providerID".to_string(), json!(provider_id));
    obj.insert(
        "api".to_string(),
        json!({
            "id": api_id,
            "npm": provider_npm(provider_id),
            "url": "",
        }),
    );
    obj.insert(
        "capabilities".to_string(),
        json!({
            "temperature": model.temperature.unwrap_or(false),
            "reasoning": model.reasoning.unwrap_or(false),
            "attachment": model.attachment.unwrap_or(false),
            "toolcall": model.tool_call.unwrap_or(true),
            "input": {
                "text": model
                    .modalities
                    .as_ref()
                    .map(|m| m.input.iter().any(|item| item == "text"))
                    .unwrap_or(true),
                "audio": model
                    .modalities
                    .as_ref()
                    .map(|m| m.input.iter().any(|item| item == "audio"))
                    .unwrap_or(false),
                "image": model
                    .modalities
                    .as_ref()
                    .map(|m| m.input.iter().any(|item| item == "image"))
                    .unwrap_or(model.attachment.unwrap_or(false)),
                "video": model
                    .modalities
                    .as_ref()
                    .map(|m| m.input.iter().any(|item| item == "video"))
                    .unwrap_or(false),
                "pdf": model
                    .modalities
                    .as_ref()
                    .map(|m| m.input.iter().any(|item| item == "pdf"))
                    .unwrap_or(false),
            },
            "output": {
                "text": model
                    .modalities
                    .as_ref()
                    .map(|m| m.output.iter().any(|item| item == "text"))
                    .unwrap_or(true),
                "audio": model
                    .modalities
                    .as_ref()
                    .map(|m| m.output.iter().any(|item| item == "audio"))
                    .unwrap_or(false),
                "image": model
                    .modalities
                    .as_ref()
                    .map(|m| m.output.iter().any(|item| item == "image"))
                    .unwrap_or(false),
                "video": model
                    .modalities
                    .as_ref()
                    .map(|m| m.output.iter().any(|item| item == "video"))
                    .unwrap_or(false),
                "pdf": model
                    .modalities
                    .as_ref()
                    .map(|m| m.output.iter().any(|item| item == "pdf"))
                    .unwrap_or(false),
            },
            "interleaved": false,
        }),
    );
    obj.insert("cost".to_string(), public_model_config_cost(model));
    obj.insert("limit".to_string(), public_model_config_limit(model));
    value
}

fn public_model_config_cost(model: &ModelConfig) -> Value {
    let Some(cost) = model.cost.as_ref() else {
        return json!({ "input": 0, "output": 0, "cache": { "read": 0, "write": 0 } });
    };
    json!({
        "input": cost.input,
        "output": cost.output,
        "cache": {
            "read": cost.cache_read.unwrap_or(0.0),
            "write": cost.cache_write.unwrap_or(0.0),
        },
    })
}

fn public_model_config_limit(model: &ModelConfig) -> Value {
    let Some(limit) = model.limit.as_ref() else {
        return json!({ "context": 0, "output": 0 });
    };
    json!({
        "context": limit.context,
        "input": limit.input,
        "output": limit.output,
    })
}

fn provider_npm(provider_id: &str) -> &'static str {
    match provider_id {
        "anthropic" => "@ai-sdk/anthropic",
        "google" => "@ai-sdk/google",
        "xai" => "@ai-sdk/xai",
        "mistral" => "@ai-sdk/mistral",
        "groq" => "@ai-sdk/groq",
        "deepinfra" => "@ai-sdk/deepinfra",
        "cerebras" => "@ai-sdk/cerebras",
        "cohere" => "@ai-sdk/cohere",
        "togetherai" => "@ai-sdk/togetherai",
        "perplexity" => "@ai-sdk/perplexity",
        "vercel" => "@ai-sdk/vercel",
        "alibaba" => "@ai-sdk/alibaba",
        "github-copilot" => "@ai-sdk/github-copilot",
        "openrouter" => "@openrouter/ai-sdk-provider",
        "gitlab" => "gitlab-ai-provider",
        "venice" => "venice-ai-sdk-provider",
        _ => "@ai-sdk/openai-compatible",
    }
}

fn apply_configured_providers(
    all: &mut BTreeMap<String, Value>,
    connected: &mut BTreeSet<String>,
    config: Option<&Config>,
) {
    let Some(config) = config else {
        return;
    };
    let Some(configured) = config.provider.as_ref() else {
        return;
    };

    for (id, entry) in configured {
        let mut info = all
            .remove(id)
            .unwrap_or_else(|| empty_provider_info(id, entry.name.as_deref().unwrap_or(id)));
        merge_provider_entry(&mut info, id, entry);
        all.insert(id.clone(), info);
        connected.insert(id.clone());
    }
}

fn merge_provider_entry(info: &mut Value, id: &str, entry: &ProviderConfigEntry) {
    let obj = info.as_object_mut().expect("provider serializes as object");
    obj.insert("id".to_string(), json!(id));
    obj.insert("source".to_string(), json!("config"));
    if let Some(name) = entry.name.as_ref() {
        obj.insert("name".to_string(), json!(name));
    }
    if let Some(env) = entry.env.as_ref() {
        obj.insert("env".to_string(), json!(env));
    }
    if let Some(options) = entry.options.as_ref() {
        obj.insert(
            "options".to_string(),
            serde_json::to_value(options).unwrap_or_else(|_| json!({})),
        );
    }

    if let Some(config_models) = entry.models.as_ref() {
        let models = obj
            .entry("models".to_string())
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("models serializes as object");
        for (model_id, model) in config_models {
            models.insert(
                model_id.clone(),
                public_model_from_config(id, model_id, model),
            );
        }
    }

    if let Some(whitelist) = entry.whitelist.as_ref() {
        if let Some(models) = obj.get_mut("models").and_then(Value::as_object_mut) {
            let allowed = whitelist.iter().collect::<BTreeSet<_>>();
            models.retain(|model_id, _| allowed.contains(model_id));
        }
    }
    if let Some(blacklist) = entry.blacklist.as_ref() {
        if let Some(models) = obj.get_mut("models").and_then(Value::as_object_mut) {
            for model_id in blacklist {
                models.remove(model_id);
            }
        }
    }
}

fn empty_provider_info(id: &str, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "source": "config",
        "env": [],
        "options": {},
        "models": {},
    })
}

fn apply_provider_filter(
    all: &mut BTreeMap<String, Value>,
    connected: &mut BTreeSet<String>,
    config: Option<&Config>,
) {
    let Some(config) = config else {
        return;
    };
    let enabled = config
        .enabled_providers
        .as_ref()
        .map(|items| items.iter().cloned().collect::<BTreeSet<_>>());
    let disabled = config
        .disabled_providers
        .as_ref()
        .map(|items| items.iter().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default();

    all.retain(|id, _| {
        enabled
            .as_ref()
            .map(|items| items.contains(id))
            .unwrap_or(true)
            && !disabled.contains(id)
    });
    connected.retain(|id| all.contains_key(id));
}

fn apply_environment_connections(
    all: &mut BTreeMap<String, Value>,
    connected: &mut BTreeSet<String>,
) {
    for (id, provider) in all.iter_mut() {
        let env = provider
            .get("env")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let has_env = env.iter().any(|key| {
            key.as_str()
                .and_then(|key| std::env::var(key).ok())
                .is_some_and(|value| !value.trim().is_empty())
        });
        if has_env {
            if let Some(obj) = provider.as_object_mut() {
                obj.insert("source".to_string(), json!("env"));
            }
            connected.insert(id.clone());
        }
    }
}

async fn apply_auth_connections(
    all: &mut BTreeMap<String, Value>,
    connected: &mut BTreeSet<String>,
    state: &AppState,
) -> Result<(), StatusCode> {
    let store = AuthStore::new(state.data_dir());
    store
        .load()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    for (id, auth) in store.all().await {
        let Some(provider) = all.get_mut(&id) else {
            continue;
        };
        if let Some(obj) = provider.as_object_mut() {
            obj.insert("source".to_string(), json!(auth_source(&auth)));
        }
        connected.insert(id);
    }
    Ok(())
}

fn auth_source(auth: &AuthInfo) -> &'static str {
    match auth {
        AuthInfo::Api { .. } => "api",
        AuthInfo::Oauth { .. } => "api",
        AuthInfo::Wellknown { .. } => "custom",
    }
}

fn apply_state_provider(
    all: &mut BTreeMap<String, Value>,
    connected: &mut BTreeSet<String>,
    state: &AppState,
) {
    let Some(provider) = state.provider.as_ref() else {
        return;
    };
    let id = provider.name().to_string();
    if !provider_allowed(&id, state.config.as_ref()) {
        return;
    }
    all.entry(id.clone()).or_insert_with(|| {
        json!({
            "id": id,
            "name": provider.name(),
            "source": "custom",
            "env": [],
            "options": {},
            "models": model_record(provider.name(), provider.models()),
        })
    });
    connected.insert(provider.name().to_string());
}

fn provider_allowed(id: &str, config: Option<&Config>) -> bool {
    let Some(config) = config else {
        return true;
    };
    if config
        .enabled_providers
        .as_ref()
        .is_some_and(|enabled| !enabled.iter().any(|item| item == id))
    {
        return false;
    }
    !config
        .disabled_providers
        .as_ref()
        .is_some_and(|disabled| disabled.iter().any(|item| item == id))
}

fn default_model_ids(providers: &BTreeMap<String, Value>) -> BTreeMap<String, String> {
    let mut defaults = BTreeMap::new();
    for (provider_id, provider) in providers {
        let Some(models) = provider.get("models").and_then(Value::as_object) else {
            continue;
        };
        let Some((model_id, model)) = models.iter().next() else {
            continue;
        };
        defaults.insert(
            provider_id.clone(),
            model
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(model_id)
                .to_string(),
        );
    }
    defaults
}

fn provider_auth_method_map() -> Value {
    json!({
        "openai": [
            {
                "type": "api",
                "label": "API key"
            }
        ],
        "azure": [
            {
                "type": "api",
                "label": "API key",
                "prompts": [
                    {
                        "type": "text",
                        "key": "resourceName",
                        "message": "Enter Azure Resource Name",
                        "placeholder": "e.g. my-models"
                    }
                ]
            }
        ]
    })
}
