use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub struct StatusBar {
    agent: String,
    model: Option<String>,
    status_text: String,
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            agent: "build".to_string(),
            model: None,
            status_text: "Ready".to_string(),
        }
    }

    pub fn set_agent(&mut self, agent: String) {
        self.agent = agent;
    }

    pub fn set_model(&mut self, model: Option<String>) {
        self.model = model;
    }

    pub fn set_status(&mut self, status: String) {
        self.status_text = status;
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        let model_text = self.model.clone().unwrap_or_else(|| "default".to_string());
        let text = format!(
            "Agent: {} | Model: {} | {}",
            self.agent, model_text, self.status_text
        );

        let paragraph = Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(Color::White).bg(Color::DarkGray),
        )));

        f.render_widget(paragraph, area);
    }
}
