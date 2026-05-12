use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    Terminal,
};

use crate::tui::state::{AppState, StateStore};
use crate::tui::components::{Chat, HelpOverlay, PromptInput, Sidebar, StatusBar, Toast};
use crate::tui::event::TuiEvent;
use crate::tui::keymap::KeyMap;
use crate::id::SessionID;
use crate::message::{Message, WithParts};

pub struct App {
    state: StateStore,
    sidebar: Sidebar,
    chat: Option<Chat>,
    prompt: PromptInput,
    status_bar: StatusBar,
    help: HelpOverlay,
    toast: Toast,
    keymap: KeyMap,
    should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            state: StateStore::new(),
            sidebar: Sidebar::new(),
            chat: None,
            prompt: PromptInput::new(),
            status_bar: StatusBar::new(),
            help: HelpOverlay::new(),
            toast: Toast::new(),
            keymap: KeyMap::new(),
            should_quit: false,
        }
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let res = self.run_loop(&mut terminal);

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        res
    }

    fn run_loop<B: ratatui::backend::Backend>(&mut self, terminal: &mut Terminal<B>) -> anyhow::Result<()> {
        loop {
            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key);
                }
            }

            self.toast.tick();

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        let action = self.keymap.get_action(key);

        match action {
            Some("quit") => {
                self.should_quit = true;
            }
            Some("submit") => {
                let text = self.prompt.submit();
                if !text.is_empty() {
                    self.toast.show(format!("Sending: {}", text));
                    self.state.write().set_toast(Some(format!("Processing: {}", text)));
                    self.status_bar.set_status(format!("Processing: {}", text));
                    
                    if let Some(chat) = &mut self.chat {
                        let session_id = chat.session_id.clone();
                        let user_msg = crate::message::Message::User(crate::message::UserMessage::default());
                        let with_parts = WithParts {
                            info: user_msg.clone(),
                            parts: vec![crate::message::Part::Text(crate::message::part::TextPart {
                                id: crate::id::PartID::new(),
                                session_id: session_id.clone(),
                                message_id: crate::id::MessageID::new(),
                                text: text.clone(),
                                synthetic: None,
                                ignored: None,
                                time: None,
                                metadata: None,
                            })],
                        };
                        chat.add_message(with_parts);
                        self.state.write().add_message(session_id, user_msg);
                    }
                }
            }
            Some("cancel") => {
                self.prompt.clear();
            }
            Some("help") => {
                self.help.toggle();
            }
            Some("next_panel") => {
                let state = self.state.read();
                if state.sidebar_visible() {
                    self.sidebar.select_next();
                }
            }
            Some("prev_panel") => {
                let state = self.state.read();
                if state.sidebar_visible() {
                    self.sidebar.select_prev();
                }
            }
            Some("scroll_up") => {
                self.state.write().scroll_up(1);
            }
            Some("scroll_down") => {
                self.state.write().scroll_down(1);
            }
            Some("scroll_page_up") => {
                self.state.write().scroll_up(10);
            }
            Some("scroll_page_down") => {
                self.state.write().scroll_down(10);
            }
            None => {
                if !self.help.is_visible() {
                    if let KeyCode::Char(c) = key.code {
                        if key.modifiers == KeyModifiers::NONE {
                            self.prompt.append(c);
                        }
                    } else if let KeyCode::Backspace = key.code {
                        self.prompt.backspace();
                    }
                }
            }
            _ => {}
        }
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let size = f.size();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(0)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(3),
            ].as_ref())
            .split(size);

        self.status_bar.render(f, chunks[0]);

        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .margin(0)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(80),
            ].as_ref())
            .split(chunks[1]);

        let state = self.state.read();
        let sidebar_visible = state.sidebar_visible();
        self.sidebar.render(f, main_chunks[0], sidebar_visible);

        let chat_area = if sidebar_visible {
            main_chunks[1]
        } else {
            chunks[1]
        };

        if let Some(chat) = &self.chat {
            chat.render(f, chat_area, state.scroll_offset());
        } else {
            let block = ratatui::widgets::Block::default()
                .title("No session selected")
                .borders(ratatui::widgets::Borders::ALL);
            f.render_widget(block, chat_area);
        }

        if self.help.is_visible() {
            let help_area = ratatui::layout::Rect::new(
                size.x + 2,
                size.y + 2,
                size.width.saturating_sub(4),
                size.height.saturating_sub(4),
            );
            self.help.render(f, help_area);
        }

        self.toast.render(f, size);
    }

    pub fn load_sessions(&mut self, sessions: Vec<crate::session::SessionRow>) {
        let sessions_clone = sessions.clone();
        self.sidebar.set_sessions(sessions);
        self.state.write().set_sessions(sessions_clone);
    }

    pub fn select_session(&mut self, session_id: SessionID) {
        self.chat = Some(Chat::new(session_id.clone()));
        self.state.write().set_selected_session(Some(session_id.clone()));
        self.status_bar.set_status(format!("Session: {}", session_id));
        
        self.toast.show(format!("Session selected: {}", session_id));
    }

    pub fn load_messages(&mut self, session_id: &SessionID, messages: Vec<WithParts>) {
        if let Some(chat) = &mut self.chat {
            if chat.session_id == *session_id {
                chat.set_messages(messages);
            }
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}