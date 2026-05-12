use crossterm::event::{Event, KeyEvent, MouseEvent};

#[derive(Debug, Clone)]
pub enum TuiEvent {
    PromptAppend(String),
    PromptSubmit(String),
    CommandExecute(String),
    ToastShow(String),
    ToastClear,
    SessionSelect(String),
    SessionCreate(String),
    HelpToggle,
    SidebarToggle,
    Quit,
    ScrollUp(usize),
    ScrollDown(usize),
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
}

impl TuiEvent {
    pub fn from_crossterm(event: Event) -> Option<Self> {
        match event {
            Event::Key(key) => Some(TuiEvent::Key(key)),
            Event::Mouse(mouse) => Some(TuiEvent::Mouse(mouse)),
            Event::Resize(cols, rows) => Some(TuiEvent::Resize(cols, rows)),
            _ => None,
        }
    }

    pub fn is_quit(&self) -> bool {
        matches!(self, TuiEvent::Quit)
    }
}