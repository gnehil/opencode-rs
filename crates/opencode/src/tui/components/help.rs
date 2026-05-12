use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::tui::keymap::KeyMap;

pub struct HelpOverlay {
    visible: bool,
}

impl HelpOverlay {
    pub fn new() -> Self {
        Self { visible: false }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }

        f.render_widget(Clear, area);

        let block = Block::default()
            .title("Help - Press 'h' to close")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Yellow));

        let inner = block.inner(area);
        f.render_widget(block, area);

        let keymap = KeyMap::new();
        let lines: Vec<Line> = keymap.bindings().iter().map(|b| {
            let key_text = match b.key {
                crossterm::event::KeyCode::Char(c) => format!("{}", c),
                crossterm::event::KeyCode::Enter => "Enter".to_string(),
                crossterm::event::KeyCode::Esc => "Esc".to_string(),
                crossterm::event::KeyCode::Tab => "Tab".to_string(),
                crossterm::event::KeyCode::Up => "Up".to_string(),
                crossterm::event::KeyCode::Down => "Down".to_string(),
                crossterm::event::KeyCode::PageUp => "PgUp".to_string(),
                crossterm::event::KeyCode::PageDown => "PgDn".to_string(),
                _ => "?".to_string(),
            };

            let mod_text = if b.modifiers == crossterm::event::KeyModifiers::CONTROL {
                "Ctrl+"
            } else {
                ""
            };

            Line::from(vec![
                Span::styled(format!("{}{}: ", mod_text, key_text), Style::default().fg(Color::Cyan)),
                Span::styled(b.action.clone(), Style::default().fg(Color::White)),
            ])
        }).collect();

        let paragraph = Paragraph::new(lines);
        f.render_widget(paragraph, inner);
    }
}