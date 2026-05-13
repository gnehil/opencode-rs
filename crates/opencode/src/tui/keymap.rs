use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone)]
pub struct KeyBinding {
    pub key: KeyCode,
    pub modifiers: KeyModifiers,
    pub action: String,
}

pub struct KeyMap {
    bindings: Vec<KeyBinding>,
}

impl KeyMap {
    pub fn new() -> Self {
        Self {
            bindings: vec![
                KeyBinding {
                    key: KeyCode::Char('q'),
                    modifiers: KeyModifiers::NONE,
                    action: "quit".to_string(),
                },
                KeyBinding {
                    key: KeyCode::Char('c'),
                    modifiers: KeyModifiers::CONTROL,
                    action: "quit".to_string(),
                },
                KeyBinding {
                    key: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                    action: "submit".to_string(),
                },
                KeyBinding {
                    key: KeyCode::Esc,
                    modifiers: KeyModifiers::NONE,
                    action: "cancel".to_string(),
                },
                KeyBinding {
                    key: KeyCode::Tab,
                    modifiers: KeyModifiers::NONE,
                    action: "next_panel".to_string(),
                },
                KeyBinding {
                    key: KeyCode::BackTab,
                    modifiers: KeyModifiers::NONE,
                    action: "prev_panel".to_string(),
                },
                KeyBinding {
                    key: KeyCode::Char('h'),
                    modifiers: KeyModifiers::NONE,
                    action: "help".to_string(),
                },
                KeyBinding {
                    key: KeyCode::Char('n'),
                    modifiers: KeyModifiers::CONTROL,
                    action: "new_session".to_string(),
                },
                KeyBinding {
                    key: KeyCode::Up,
                    modifiers: KeyModifiers::NONE,
                    action: "scroll_up".to_string(),
                },
                KeyBinding {
                    key: KeyCode::Down,
                    modifiers: KeyModifiers::NONE,
                    action: "scroll_down".to_string(),
                },
                KeyBinding {
                    key: KeyCode::PageUp,
                    modifiers: KeyModifiers::NONE,
                    action: "scroll_page_up".to_string(),
                },
                KeyBinding {
                    key: KeyCode::PageDown,
                    modifiers: KeyModifiers::NONE,
                    action: "scroll_page_down".to_string(),
                },
            ],
        }
    }

    pub fn get_action(&self, key: KeyEvent) -> Option<&str> {
        self.bindings
            .iter()
            .find(|b| b.key == key.code && b.modifiers == key.modifiers)
            .map(|b| b.action.as_str())
    }

    pub fn bindings(&self) -> &[KeyBinding] {
        &self.bindings
    }
}

impl Default for KeyMap {
    fn default() -> Self {
        Self::new()
    }
}
