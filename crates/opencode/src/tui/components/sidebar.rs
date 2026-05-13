use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::session::SessionRow;

pub struct Sidebar {
    sessions: Vec<SessionRow>,
    selected_index: usize,
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            selected_index: 0,
        }
    }

    pub fn set_sessions(&mut self, sessions: Vec<SessionRow>) {
        self.sessions = sessions;
        if self.selected_index >= self.sessions.len() && !self.sessions.is_empty() {
            self.selected_index = 0;
        }
    }

    pub fn selected(&self) -> Option<&SessionRow> {
        self.sessions.get(self.selected_index)
    }

    pub fn select_next(&mut self) {
        if self.selected_index < self.sessions.len().saturating_sub(1) {
            self.selected_index += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn render(&self, f: &mut Frame, area: Rect, visible: bool) {
        if !visible {
            return;
        }

        let block = Block::default()
            .title("Sessions")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Yellow));

        let inner = block.inner(area);
        f.render_widget(block, area);

        let items: Vec<ListItem> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(i, session)| {
                let style = if i == self.selected_index {
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };
                let title = session.title.chars().take(30).collect::<String>();
                ListItem::new(Line::from(Span::styled(title, style)))
            })
            .collect();

        let list = List::new(items);
        f.render_widget(list, inner);
    }

    pub fn is_visible(&self, sidebar_visible: bool) -> bool {
        sidebar_visible
    }
}
