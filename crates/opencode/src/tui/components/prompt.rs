use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub struct PromptInput {
    text: String,
    cursor_position: usize,
}

impl PromptInput {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor_position: 0,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor_position = self.text.len();
    }

    pub fn append(&mut self, ch: char) {
        self.text.insert(self.cursor_position, ch);
        self.cursor_position += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor_position > 0 {
            self.text.remove(self.cursor_position - 1);
            self.cursor_position -= 1;
        }
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor_position = 0;
    }

    pub fn submit(&mut self) -> String {
        let text = self.text.clone();
        self.clear();
        text
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Prompt (Enter to submit)")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Magenta));

        let inner = block.inner(area);
        f.render_widget(block, area);

        let display_text = if self.text.is_empty() {
            "Type your message..."
        } else {
            &self.text
        };

        let paragraph = Paragraph::new(Line::from(Span::raw(display_text)))
            .style(Style::default().fg(Color::White));

        f.render_widget(paragraph, inner);
    }
}