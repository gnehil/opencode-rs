use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

pub struct Toast {
    message: Option<String>,
    timer: u32,
}

impl Toast {
    pub fn new() -> Self {
        Self {
            message: None,
            timer: 0,
        }
    }

    pub fn show(&mut self, message: String) {
        self.message = Some(message);
        self.timer = 50;
    }

    pub fn tick(&mut self) {
        if self.timer > 0 {
            self.timer -= 1;
            if self.timer == 0 {
                self.message = None;
            }
        }
    }

    pub fn is_visible(&self) -> bool {
        self.message.is_some() && self.timer > 0
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        if !self.is_visible() {
            return;
        }

        let toast_area = Rect::new(area.x, area.y, area.width, 1);
        f.render_widget(Clear, toast_area);

        let paragraph = Paragraph::new(Line::from(Span::styled(
            self.message.clone().unwrap_or_default(),
            Style::default().fg(Color::White).bg(Color::Blue),
        )));

        f.render_widget(paragraph, toast_area);
    }
}