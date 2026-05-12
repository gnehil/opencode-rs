//! ACP (Agent Client Protocol) type definitions
//! Based on the Agent Client Protocol specification v1

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// ACP Session State - tracks session info for ACP protocol
#[derive(Debug, Clone)]
pub struct ACPSessionState {
    /// Unique session ID
    pub id: String,
    /// Working directory for this session
    pub cwd: String,
    /// MCP server configurations
    pub mcp_servers: Vec<McpServerConfig>,
    /// Creation timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Model selection (provider/model)
    pub model: Option<ModelSelection>,
    /// Model variant (e.g., "high", "max")
    pub variant: Option<String>,
    /// Agent mode ID
    pub mode_id: Option<String>,
}

/// Model selection with provider and model IDs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSelection {
    pub provider_id: String,
    pub model_id: String,
}

/// MCP Server Configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpServerConfig {
    #[serde(rename = "remote")]
    Remote {
        name: String,
        url: String,
        headers: HashMap<String, String>,
    },
    #[serde(rename = "local")]
    Local {
        name: String,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
}

/// JSON-RPC 2.0 Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<RequestId>,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// Request ID can be number or string
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

/// JSON-RPC Error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ACP Protocol Types

/// Initialize Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeRequest {
    pub protocol_version: i32,
    #[serde(default)]
    pub client_capabilities: Option<ClientCapabilities>,
}

/// Initialize Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResponse {
    pub protocol_version: i32,
    pub agent_capabilities: AgentCapabilities,
    pub auth_methods: Vec<AuthMethod>,
    pub agent_info: AgentInfo,
}

/// Client Capabilities
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub _meta: Option<HashMap<String, serde_json::Value>>,
}

/// Agent Capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilities {
    pub load_session: bool,
    pub mcp_capabilities: McpCapabilities,
    pub prompt_capabilities: PromptCapabilities,
    pub session_capabilities: SessionCapabilities,
}

/// MCP Capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCapabilities {
    pub http: bool,
    pub sse: bool,
}

/// Prompt Capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptCapabilities {
    pub embedded_context: bool,
    pub image: bool,
}

/// Session Capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCapabilities {
    pub close: SessionCloseCapabilities,
    pub fork: SessionForkCapabilities,
    pub list: SessionListCapabilities,
    pub resume: SessionResumeCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCloseCapabilities {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionForkCapabilities {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListCapabilities {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResumeCapabilities {}

/// Auth Method
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub _meta: Option<HashMap<String, serde_json::Value>>,
}

/// Agent Info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub name: String,
    pub version: String,
}

// Session Management Types

/// New Session Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewSessionRequest {
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
}

/// ACP McpServer (from SDK)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpServer {
    #[serde(rename = "sse")]
    Sse {
        name: String,
        url: String,
        headers: Vec<HeaderEntry>,
    },
    #[serde(rename = "stdio")]
    Stdio {
        name: String,
        command: String,
        args: Vec<String>,
        env: Vec<EnvEntry>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderEntry {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvEntry {
    pub name: String,
    pub value: String,
}

/// New Session Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewSessionResponse {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_options: Option<Vec<SessionConfigOption>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<ModelsInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<ModesInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub _meta: Option<HashMap<String, serde_json::Value>>,
}

/// Models Info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsInfo {
    pub current_model_id: String,
    pub available_models: Vec<ModelOption>,
}

/// Model Option
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelOption {
    pub model_id: String,
    pub name: String,
}

/// Modes Info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModesInfo {
    pub available_modes: Vec<ModeOption>,
    pub current_mode_id: String,
}

/// Mode Option
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeOption {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Session Config Option
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfigOption {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub category: String,
    pub type_: String,
    pub current_value: String,
    pub options: Vec<ConfigOptionValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigOptionValue {
    pub value: String,
    pub name: String,
}

/// Load Session Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadSessionRequest {
    pub session_id: String,
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
}

/// List Sessions Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSessionsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// List Sessions Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSessionsResponse {
    pub sessions: Vec<SessionInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Session Info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub updated_at: String,
}

/// Close Session Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseSessionRequest {
    pub session_id: String,
}

/// Close Session Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseSessionResponse {}

/// Fork Session Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkSessionRequest {
    pub session_id: String,
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
}

/// Resume Session Request  
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeSessionRequest {
    pub session_id: String,
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
}

/// Set Session Model Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetSessionModelRequest {
    pub session_id: String,
    pub model_id: String,
}

/// Set Session Mode Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetSessionModeRequest {
    pub session_id: String,
    pub mode_id: String,
}

/// Set Session Config Option Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetSessionConfigOptionRequest {
    pub session_id: String,
    pub config_id: String,
    pub value: serde_json::Value,
}

/// Set Session Config Option Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetSessionConfigOptionResponse {
    pub config_options: Vec<SessionConfigOption>,
}

// Prompt Types

/// Prompt Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptRequest {
    pub session_id: String,
    pub prompt: Vec<PromptContent>,
}

/// Prompt Content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PromptContent {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        annotations: Option<Annotations>,
    },
    Image {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
        mime_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<String>,
    },
    Resource {
        resource: ResourceContent,
    },
    ResourceLink {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotations {
    pub audience: Vec<Role>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Role {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "assistant")]
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ResourceContent {
    Text {
        uri: String,
        mime_type: String,
        text: String,
    },
    Blob {
        uri: String,
        mime_type: String,
        blob: String,
    },
}

/// Prompt Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptResponse {
    pub stop_reason: StopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub _meta: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StopReason {
    #[serde(rename = "end_turn")]
    EndTurn,
    #[serde(rename = "tool_use")]
    ToolUse,
    #[serde(rename = "stop_sequence")]
    StopSequence,
}

/// Usage Stats
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_read_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_write_tokens: Option<i64>,
}

/// Cancel Notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelNotification {
    pub session_id: String,
}

// Session Update Types (for notifications)

/// Session Update Notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUpdate {
    pub session_id: String,
    pub update: SessionUpdateType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub enum SessionUpdateType {
    AgentMessageChunk {
        message_id: String,
        content: TextContent,
    },
    AgentThoughtChunk {
        message_id: String,
        content: TextContent,
    },
    UserMessageChunk {
        message_id: String,
        content: ContentBlock,
    },
    ToolCall {
        tool_call_id: String,
        title: String,
        kind: ToolKind,
        status: ToolCallStatus,
        locations: Vec<Location>,
        raw_input: serde_json::Value,
    },
    ToolCallUpdate {
        tool_call_id: String,
        status: ToolCallStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<ToolKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locations: Option<Vec<Location>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_input: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_output: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        content: Vec<ToolCallContent>,
    },
    UsageUpdate {
        used: i64,
        size: i64,
        cost: CostInfo,
    },
    Plan {
        entries: Vec<PlanEntry>,
    },
    ConfigOptionUpdate {
        config_options: Vec<SessionConfigOption>,
    },
    AvailableCommandsUpdate {
        available_commands: Vec<CommandInfo>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub type_: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Annotations>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentBlock {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        annotations: Option<Annotations>,
    },
    Image {
        mime_type: String,
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
    },
    Resource {
        resource: ResourceData,
    },
    ResourceLink {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ResourceData {
    Text {
        uri: String,
        mime_type: String,
        text: String,
    },
    Blob {
        uri: String,
        mime_type: String,
        blob: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolKind {
    #[serde(rename = "execute")]
    Execute,
    #[serde(rename = "fetch")]
    Fetch,
    #[serde(rename = "edit")]
    Edit,
    #[serde(rename = "search")]
    Search,
    #[serde(rename = "read")]
    Read,
    #[serde(rename = "other")]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolCallStatus {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "in_progress")]
    InProgress,
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolCallContent {
    Content {
        content: ContentBlock,
    },
    Diff {
        path: String,
        old_text: String,
        new_text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostInfo {
    pub amount: f64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanEntry {
    #[serde(default)]
    pub priority: String,
    pub status: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInfo {
    pub name: String,
    pub description: String,
}

// Permission Types

/// Request Permission Request (from agent to client)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestPermissionRequest {
    pub session_id: String,
    pub tool_call: ToolCallInfo,
    pub options: Vec<PermissionOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub tool_call_id: String,
    pub status: ToolCallStatus,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ToolKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<Location>,
    pub raw_input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionOption {
    pub option_id: String,
    pub kind: PermissionKind,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PermissionKind {
    #[serde(rename = "allow_once")]
    AllowOnce,
    #[serde(rename = "allow_always")]
    AllowAlways,
    #[serde(rename = "reject_once")]
    RejectOnce,
}

/// Permission Response (from client to agent)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionResponse {
    pub outcome: PermissionOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionOutcome {
    pub outcome: String,
    pub option_id: String,
}

/// Write Text File Request (from agent to client)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteTextFileRequest {
    pub session_id: String,
    pub path: String,
    pub content: String,
}

// Error codes
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
pub const AUTH_REQUIRED: i64 = 1;

/// Create a JSON-RPC error response
pub fn error_response(id: Option<RequestId>, code: i64, message: String) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message,
            data: None,
        }),
    }
}

/// Create a JSON-RPC success response
pub fn success_response(id: Option<RequestId>, result: serde_json::Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(result),
        error: None,
    }
}