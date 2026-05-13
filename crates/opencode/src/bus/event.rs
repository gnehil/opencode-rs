//! Event type definitions for the EventBus system.

use serde::{Deserialize, Serialize};

pub type EventId = String;
pub type HandlerId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEvent {
    pub session_id: String,
    pub message_id: String,
    pub role: MessageRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageStreamEvent {
    pub session_id: String,
    pub message_id: String,
    pub delta: String,
    pub is_done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStartEvent {
    pub session_id: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub tool_call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCompleteEvent {
    pub session_id: String,
    pub tool_name: String,
    pub tool_output: serde_json::Value,
    pub tool_call_id: String,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolErrorEvent {
    pub session_id: String,
    pub tool_name: String,
    pub error: String,
    pub tool_call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConnectedEvent {
    pub server_name: String,
    pub server_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpDisconnectedEvent {
    pub server_name: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolsChangedEvent {
    pub server_name: String,
    pub tools_added: Vec<String>,
    pub tools_removed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionAskedEvent {
    pub session_id: String,
    pub permission_id: String,
    pub permission_type: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePartUpdatedEvent {
    pub session_id: String,
    pub message_id: String,
    pub part_id: String,
    pub part_type: String,
    pub part: MessagePartData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePartData {
    pub id: String,
    #[serde(rename = "type")]
    pub part_type: String,
    pub session_id: String,
    pub message_id: String,
    pub call_id: Option<String>,
    pub tool: Option<String>,
    pub state: PartStateData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartStateData {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<AttachmentData>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentData {
    pub mime: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePartDeltaEvent {
    pub session_id: String,
    pub message_id: String,
    pub part_id: String,
    pub field: String,
    pub delta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiPromptAppendEvent {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiCommandExecuteEvent {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiToastShowEvent {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub message: String,
    pub variant: String,
    pub duration: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiSessionSelectEvent {
    pub id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum Event {
    SessionCreate(SessionEvent),
    SessionUpdate(SessionEvent),
    SessionDelete(SessionEvent),

    MessageCreate(MessageEvent),
    MessageStream(MessageStreamEvent),

    ToolStart(ToolStartEvent),
    ToolComplete(ToolCompleteEvent),
    ToolError(ToolErrorEvent),

    McpConnected(McpConnectedEvent),
    McpDisconnected(McpDisconnectedEvent),
    McpToolsChanged(McpToolsChangedEvent),

    PermissionAsked(PermissionAskedEvent),
    MessagePartUpdated(MessagePartUpdatedEvent),
    MessagePartDelta(MessagePartDeltaEvent),

    TuiPromptAppend(TuiPromptAppendEvent),
    TuiCommandExecute(TuiCommandExecuteEvent),
    TuiToastShow(TuiToastShowEvent),
    TuiSessionSelect(TuiSessionSelectEvent),
}

impl Event {
    pub fn session_id(&self) -> String {
        match self {
            Event::SessionCreate(e) => e.session_id.clone(),
            Event::SessionUpdate(e) => e.session_id.clone(),
            Event::SessionDelete(e) => e.session_id.clone(),
            Event::MessageCreate(e) => e.session_id.clone(),
            Event::MessageStream(e) => e.session_id.clone(),
            Event::ToolStart(e) => e.session_id.clone(),
            Event::ToolComplete(e) => e.session_id.clone(),
            Event::ToolError(e) => e.session_id.clone(),
            Event::McpConnected(_) => String::new(),
            Event::McpDisconnected(_) => String::new(),
            Event::McpToolsChanged(_) => String::new(),
            Event::PermissionAsked(e) => e.session_id.clone(),
            Event::MessagePartUpdated(e) => e.session_id.clone(),
            Event::MessagePartDelta(e) => e.session_id.clone(),
            Event::TuiPromptAppend(_) => String::new(),
            Event::TuiCommandExecute(_) => String::new(),
            Event::TuiToastShow(_) => String::new(),
            Event::TuiSessionSelect(e) => e.session_id.clone(),
        }
    }

    pub fn id(&self) -> EventId {
        match self {
            Event::SessionCreate(e) => e.session_id.clone(),
            Event::SessionUpdate(e) => e.session_id.clone(),
            Event::SessionDelete(e) => e.session_id.clone(),
            Event::MessageCreate(e) => e.message_id.clone(),
            Event::MessageStream(e) => e.message_id.clone(),
            Event::ToolStart(e) => e.tool_call_id.clone(),
            Event::ToolComplete(e) => e.tool_call_id.clone(),
            Event::ToolError(e) => e.tool_call_id.clone(),
            Event::McpConnected(e) => e.server_name.clone(),
            Event::McpDisconnected(e) => e.server_name.clone(),
            Event::McpToolsChanged(e) => e.server_name.clone(),
            Event::PermissionAsked(e) => e.permission_id.clone(),
            Event::MessagePartUpdated(e) => e.part_id.clone(),
            Event::MessagePartDelta(e) => e.part_id.clone(),
            Event::TuiPromptAppend(e) => e.id.clone(),
            Event::TuiCommandExecute(e) => e.id.clone(),
            Event::TuiToastShow(e) => e.id.clone(),
            Event::TuiSessionSelect(e) => e.id.clone(),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Event::SessionCreate(_) => "session.create",
            Event::SessionUpdate(_) => "session.update",
            Event::SessionDelete(_) => "session.delete",
            Event::MessageCreate(_) => "message.create",
            Event::MessageStream(_) => "message.stream",
            Event::ToolStart(_) => "tool.start",
            Event::ToolComplete(_) => "tool.complete",
            Event::ToolError(_) => "tool.error",
            Event::McpConnected(_) => "mcp.connected",
            Event::McpDisconnected(_) => "mcp.disconnected",
            Event::McpToolsChanged(_) => "mcp.tools_changed",
            Event::PermissionAsked(_) => "permission.asked",
            Event::MessagePartUpdated(_) => "message.part.updated",
            Event::MessagePartDelta(_) => "message.part.delta",
            Event::TuiPromptAppend(_) => "tui.prompt.append",
            Event::TuiCommandExecute(_) => "tui.command.execute",
            Event::TuiToastShow(_) => "tui.toast.show",
            Event::TuiSessionSelect(_) => "tui.session.select",
        }
    }

    pub fn session_create(session_id: impl Into<String>) -> Self {
        Event::SessionCreate(SessionEvent {
            session_id: session_id.into(),
            metadata: None,
        })
    }

    pub fn session_update(session_id: impl Into<String>) -> Self {
        Event::SessionUpdate(SessionEvent {
            session_id: session_id.into(),
            metadata: None,
        })
    }

    pub fn session_delete(session_id: impl Into<String>) -> Self {
        Event::SessionDelete(SessionEvent {
            session_id: session_id.into(),
            metadata: None,
        })
    }

    pub fn message_create(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        role: MessageRole,
    ) -> Self {
        Event::MessageCreate(MessageEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            role,
            content: None,
        })
    }

    pub fn message_stream(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        delta: impl Into<String>,
    ) -> Self {
        Event::MessageStream(MessageStreamEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            delta: delta.into(),
            is_done: false,
        })
    }

    pub fn tool_start(
        session_id: impl Into<String>,
        tool_name: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        Event::ToolStart(ToolStartEvent {
            session_id: session_id.into(),
            tool_name: tool_name.into(),
            tool_input: input,
            tool_call_id: uuid::Uuid::new_v4().to_string(),
        })
    }

    pub fn tool_complete(
        session_id: impl Into<String>,
        tool_name: impl Into<String>,
        output: serde_json::Value,
    ) -> Self {
        Event::ToolComplete(ToolCompleteEvent {
            session_id: session_id.into(),
            tool_name: tool_name.into(),
            tool_output: output,
            tool_call_id: uuid::Uuid::new_v4().to_string(),
            duration_ms: None,
        })
    }

    pub fn tool_error(
        session_id: impl Into<String>,
        tool_name: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Event::ToolError(ToolErrorEvent {
            session_id: session_id.into(),
            tool_name: tool_name.into(),
            error: error.into(),
            tool_call_id: uuid::Uuid::new_v4().to_string(),
        })
    }

    pub fn mcp_connected(server_name: impl Into<String>) -> Self {
        Event::McpConnected(McpConnectedEvent {
            server_name: server_name.into(),
            server_url: None,
        })
    }

    pub fn mcp_disconnected(server_name: impl Into<String>) -> Self {
        Event::McpDisconnected(McpDisconnectedEvent {
            server_name: server_name.into(),
            reason: None,
        })
    }

    pub fn mcp_tools_changed(server_name: impl Into<String>) -> Self {
        Event::McpToolsChanged(McpToolsChangedEvent {
            server_name: server_name.into(),
            tools_added: vec![],
            tools_removed: vec![],
        })
    }

    pub fn permission_asked(
        session_id: impl Into<String>,
        permission_id: impl Into<String>,
        permission_type: impl Into<String>,
        metadata: serde_json::Value,
    ) -> Self {
        Event::PermissionAsked(PermissionAskedEvent {
            session_id: session_id.into(),
            permission_id: permission_id.into(),
            permission_type: permission_type.into(),
            tool_call_id: None,
            tool_name: None,
            metadata,
        })
    }

    pub fn message_part_updated(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        part_id: impl Into<String>,
        part_type: impl Into<String>,
        part: MessagePartData,
    ) -> Self {
        Event::MessagePartUpdated(MessagePartUpdatedEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            part_id: part_id.into(),
            part_type: part_type.into(),
            part,
        })
    }

    pub fn message_part_delta(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        part_id: impl Into<String>,
        field: impl Into<String>,
        delta: impl Into<String>,
    ) -> Self {
        Event::MessagePartDelta(MessagePartDeltaEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            part_id: part_id.into(),
            field: field.into(),
            delta: delta.into(),
        })
    }

    pub fn tui_prompt_append(text: impl Into<String>) -> Self {
        Event::TuiPromptAppend(TuiPromptAppendEvent {
            id: uuid::Uuid::new_v4().to_string(),
            text: text.into(),
        })
    }

    pub fn tui_command_execute(command: Option<impl Into<String>>) -> Self {
        Event::TuiCommandExecute(TuiCommandExecuteEvent {
            id: uuid::Uuid::new_v4().to_string(),
            command: command.map(Into::into),
        })
    }

    pub fn tui_toast_show(
        title: Option<impl Into<String>>,
        message: impl Into<String>,
        variant: impl Into<String>,
        duration: u64,
    ) -> Self {
        Event::TuiToastShow(TuiToastShowEvent {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.map(Into::into),
            message: message.into(),
            variant: variant.into(),
            duration,
        })
    }

    pub fn tui_session_select(session_id: impl Into<String>) -> Self {
        Event::TuiSessionSelect(TuiSessionSelectEvent {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.into(),
        })
    }
}
