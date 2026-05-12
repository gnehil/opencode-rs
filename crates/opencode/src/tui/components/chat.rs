use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::message::{Message, UserMessage, AssistantMessage, WithParts, Part};
use crate::id::SessionID;

pub struct Chat {
    pub session_id: SessionID,
    messages: Vec<WithParts>,
}

impl Chat {
    pub fn new(session_id: SessionID) -> Self {
        Self {
            session_id,
            messages: Vec::new(),
        }
    }

    pub fn set_messages(&mut self, messages: Vec<WithParts>) {
        self.messages = messages;
    }

    pub fn add_message(&mut self, message: WithParts) {
        self.messages.push(message);
    }

    pub fn messages(&self) -> &[WithParts] {
        &self.messages
    }

    pub fn render(&self, f: &mut Frame, area: Rect, scroll_offset: usize) {
        let block = Block::default()
            .title(format!("Session: {}", self.session_id))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White));

        let inner = block.inner(area);
        f.render_widget(block, area);

        let lines: Vec<Line> = self.messages.iter().flat_map(|msg| {
            match &msg.info {
                Message::User(user) => {
                    self.render_user_message(user, &msg.parts)
                }
                Message::Assistant(assistant) => {
                    self.render_assistant_message(assistant, &msg.parts)
                }
            }
        }).collect();

        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll_offset as u16, 0));

        f.render_widget(paragraph, inner);
    }

    fn render_user_message(&self, user: &UserMessage, _parts: &[Part]) -> Vec<Line> {
        let content = if let Some(summary) = &user.summary {
            let title = summary.title.as_deref().unwrap_or("");
            let body = summary.body.as_deref().unwrap_or("");
            if !body.is_empty() {
                format!("{}\n{}", title, body)
            } else {
                title.to_string()
            }
        } else {
            "[no content]".to_string()
        };

        let lines: Vec<Line> = content
            .lines()
            .enumerate()
            .map(|(i, line)| {
                if i == 0 {
                    Line::from(vec![
                        Span::styled("You: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::raw(line),
                    ])
                } else {
                    Line::from(Span::raw(line))
                }
            })
            .collect();

        if lines.is_empty() {
            vec![Line::from(vec![
                Span::styled("You: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("[no content]"),
            ])]
        } else {
            lines
        }
    }

    fn render_assistant_message(&self, _assistant: &AssistantMessage, parts: &[Part]) -> Vec<Line> {
        let mut lines = Vec::new();

        lines.push(Line::from(vec![
            Span::styled("Assistant: ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        ]));

        for part in parts {
            match part {
                Part::Text(text_part) => {
                    for line in text_part.text.lines() {
                        lines.push(Line::from(Span::raw(line)));
                    }
                }
                Part::Reasoning(reasoning_part) => {
                    lines.push(Line::from(vec![
                        Span::styled("[Thinking]", Style::default().fg(Color::Yellow).add_modifier(Modifier::ITALIC)),
                    ]));
                    for line in reasoning_part.text.lines() {
                        lines.push(Line::from(Span::styled(line, Style::default().fg(Color::DarkGray))));
                    }
                }
                Part::Tool(tool_part) => {
                    lines.push(Line::from(vec![
                        Span::styled(format!("[Tool: {}]", tool_part.tool), Style::default().fg(Color::Magenta)),
                    ]));
                }
                _ => {}
            }
        }

        if lines.len() == 1 {
            lines.push(Line::from(Span::raw("[no content]")));
        }

        lines
    }
}