use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::r#trait::Tool;
use super::result::ToolResult;

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

fn default_custom() -> Option<bool> {
    Some(true)
}

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
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: QuestionParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid question parameters: {}", e))?;

            // Translate the tool schema into the broker's question shape so
            // the HTTP API and the tool share one type.
            let questions: Vec<crate::question::QuestionInfo> = params
                .questions
                .into_iter()
                .map(|q| crate::question::QuestionInfo {
                    question: q.question,
                    header: q.header,
                    options: q
                        .options
                        .into_iter()
                        .map(|o| crate::question::QuestionOption {
                            label: o.label,
                            description: o.description,
                        })
                        .collect(),
                    multiple: q.multiple,
                    custom: q.custom,
                })
                .collect();

            let Some(broker) = ctx.question_broker.clone() else {
                let formatted = questions
                    .iter()
                    .enumerate()
                    .map(|(i, q)| format!("Question {}: {}", i + 1, q.question))
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(ToolResult::with_metadata(
                    format!(
                        "Questions sent to user. Waiting for response.\n\nQuestions:\n{formatted}"
                    ),
                    json!({ "questions_count": questions.len() }),
                ));
            };

            let (request_id, rx) = broker.ask(&ctx.session_id, questions.clone()).await;

            match rx.await {
                Ok(crate::question::QuestionOutcome::Replied(answers)) => {
                    let mut lines = Vec::with_capacity(answers.len());
                    for (idx, answer) in answers.iter().enumerate() {
                        let question = questions
                            .get(idx)
                            .map(|q| q.question.as_str())
                            .unwrap_or("question");
                        lines.push(format!("Q: {question}\nA: {}", answer.join(", ")));
                    }
                    let output = if lines.is_empty() {
                        "User provided an empty response.".to_string()
                    } else {
                        lines.join("\n\n")
                    };
                    Ok(ToolResult::with_metadata(
                        output,
                        json!({
                            "questions_count": questions.len(),
                            "answers": answers,
                            "request_id": request_id,
                        }),
                    ))
                }
                Ok(crate::question::QuestionOutcome::Rejected) => {
                    Err(anyhow::anyhow!("The user dismissed this question"))
                }
                Err(_) => Err(anyhow::anyhow!("Question broker dropped before reply")),
            }
        })
    }
}
