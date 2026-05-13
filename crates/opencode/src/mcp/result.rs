use rmcp::model::Content;

#[derive(Debug, Clone)]
pub enum McpContent {
    Text { text: String },
    Image { data: String, mime_type: String },
    Resource { resource: McpResourceContent },
}

#[derive(Debug, Clone)]
pub struct McpResourceContent {
    pub uri: String,
    pub mime_type: Option<String>,
    pub text: Option<String>,
    pub blob: Option<String>,
}

impl McpContent {
    pub fn from_rmcp(content: &Content) -> Self {
        use rmcp::model::RawContent;
        match &content.raw {
            RawContent::Text(raw_text) => McpContent::Text {
                text: raw_text.text.clone(),
            },
            RawContent::Image(raw_image) => McpContent::Image {
                data: raw_image.data.clone(),
                mime_type: raw_image.mime_type.clone(),
            },
            RawContent::Resource(raw_embedded) => {
                let uri = match &raw_embedded.resource {
                    rmcp::model::ResourceContents::TextResourceContents { uri, .. }
                    | rmcp::model::ResourceContents::BlobResourceContents { uri, .. } => {
                        uri.clone()
                    }
                };
                let (mime_type, text, blob) = match &raw_embedded.resource {
                    rmcp::model::ResourceContents::TextResourceContents {
                        mime_type, text, ..
                    } => (mime_type.clone(), Some(text.clone()), None),
                    rmcp::model::ResourceContents::BlobResourceContents {
                        mime_type, blob, ..
                    } => (mime_type.clone(), None, Some(blob.clone())),
                };
                McpContent::Resource {
                    resource: McpResourceContent {
                        uri,
                        mime_type,
                        text,
                        blob,
                    },
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    pub is_error: bool,
}

impl McpToolResult {
    pub fn from_rmcp(result: &rmcp::model::CallToolResult) -> Self {
        Self {
            content: result.content.iter().map(McpContent::from_rmcp).collect(),
            is_error: result.is_error.unwrap_or(false),
        }
    }
}
