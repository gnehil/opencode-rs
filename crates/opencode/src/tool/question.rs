use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::result::ToolResult;
use super::r#trait::Tool;

#[derive(Debug, Clone, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuestionPrompt {
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multiple: Option<bool>,
    #[serde(default = "default_custom")]
    pub custom: Option<bool>,
}

fn default_custom() -> Option<bool> { Some(true) }

#[derive(Debug, Deserialize)]
pub struct QuestionParams {
    pub questions: Vec<QuestionPrompt>,
}

pub struct QuestionTool;

impl Tool for QuestionTool {
    fn name(&self) -> &str {
        "question"
    }

    fn description(&self) -> &str {
        "Use this tool when you need to ask the user questions during execution."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": {
                                "type": "string",
                                "description": "Complete question"
                            },
                            "header": {
                                "type": "string",
                                "description": "Very short label (max 30 chars)"
                            },
                            "options": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {
                                            "type": "string",
                                            "description": "Display text (1-5 words)"
                                        },
                                        "description": {
                                            "type": "string",
                                            "description": "Explanation of choice"
                                        }
                                    },
                                    "required": ["label"]
                                }
                            },
                            "multiple": {
                                "type": "boolean",
                                "description": "Allow selecting multiple choices"
                            },
                            "custom": {
                                "type": "boolean",
                                "default": true,
                                "description": "Add 'Type your own answer' option"
                            }
                        },
                        "required": ["question", "options"]
                    }
                }
            },
            "required": ["questions"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: QuestionParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid question parameters: {}", e))?;

            let formatted = params.questions
                .iter()
                .enumerate()
                .map(|(i, q)| format!("Question {}: {}", i + 1, q.question))
                .collect::<Vec<_>>()
                .join("\n");

            let output = format!(
                "Questions sent to user. Waiting for response.\n\nQuestions:\n{}",
                formatted
            );

            Ok(ToolResult::with_metadata(
                output,
                json!({
                    "questions_count": params.questions.len(),
                }),
            ))
        })
    }
}