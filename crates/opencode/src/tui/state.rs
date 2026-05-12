use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::id::SessionID;
use crate::message::Message;
use crate::provider::ModelInfo;
use crate::session::SessionRow;

#[derive(Debug, Clone)]
pub enum Route {
    Home,
    Session(SessionID),
    Plugin,
}

#[derive(Debug, Clone)]
pub struct AppState {
    route: Route,
    sessions: Vec<SessionRow>,
    messages: HashMap<SessionID, Vec<Message>>,
    selected_session: Option<SessionID>,
    selected_agent: String,
    selected_model: Option<String>,
    sidebar_visible: bool,
    input_text: String,
    input_history: Vec<String>,
    toast_message: Option<String>,
    show_help: bool,
    scroll_offset: usize,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            route: Route::Home,
            sessions: Vec::new(),
            messages: HashMap::new(),
            selected_session: None,
            selected_agent: "build".to_string(),
            selected_model: None,
            sidebar_visible: true,
            input_text: String::new(),
            input_history: Vec::new(),
            toast_message: None,
            show_help: false,
            scroll_offset: 0,
        }
    }

    pub fn route(&self) -> &Route {
        &self.route
    }

    pub fn set_route(&mut self, route: Route) {
        self.route = route;
    }

    pub fn sessions(&self) -> &[SessionRow] {
        &self.sessions
    }

    pub fn set_sessions(&mut self, sessions: Vec<SessionRow>) {
        self.sessions = sessions;
    }

    pub fn messages(&self, session_id: &SessionID) -> Option<&Vec<Message>> {
        self.messages.get(session_id)
    }

    pub fn add_message(&mut self, session_id: SessionID, message: Message) {
        self.messages.entry(session_id).or_insert_with(Vec::new).push(message);
    }

    pub fn selected_session(&self) -> Option<&SessionID> {
        self.selected_session.as_ref()
    }

    pub fn set_selected_session(&mut self, session_id: Option<SessionID>) {
        self.selected_session = session_id;
    }

    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
    }

    pub fn input_text(&self) -> &str {
        &self.input_text
    }

    pub fn set_input_text(&mut self, text: String) {
        self.input_text = text;
    }

    pub fn append_input(&mut self, ch: char) {
        self.input_text.push(ch);
    }

    pub fn backspace_input(&mut self) {
        self.input_text.pop();
    }

    pub fn clear_input(&mut self) {
        self.input_text.clear();
    }

    pub fn submit_input(&mut self) -> String {
        let text = self.input_text.clone();
        if !text.is_empty() {
            self.input_history.push(text.clone());
        }
        self.input_text.clear();
        text
    }

    pub fn input_history(&self) -> &[String] {
        &self.input_history
    }

    pub fn toast_message(&self) -> Option<&str> {
        self.toast_message.as_deref()
    }

    pub fn set_toast(&mut self, message: Option<String>) {
        self.toast_message = message;
    }

    pub fn show_help(&self) -> bool {
        self.show_help
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
    }

    pub fn selected_agent(&self) -> &str {
        &self.selected_agent
    }

    pub fn set_selected_agent(&mut self, agent: String) {
        self.selected_agent = agent;
    }

    pub fn selected_model(&self) -> Option<&str> {
        self.selected_model.as_deref()
    }

    pub fn set_selected_model(&mut self, model: Option<String>) {
        self.selected_model = model;
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct StateStore {
    state: Arc<RwLock<AppState>>,
}

impl StateStore {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(AppState::new())),
        }
    }

    pub fn read(&self) -> parking_lot::RwLockReadGuard<'_, AppState> {
        self.state.read()
    }

    pub fn write(&self) -> parking_lot::RwLockWriteGuard<'_, AppState> {
        self.state.write()
    }

    pub fn clone_state(&self) -> AppState {
        self.state.read().clone()
    }
}

impl Default for StateStore {
    fn default() -> Self {
        Self::new()
    }
}