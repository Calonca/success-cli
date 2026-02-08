/// Abstract key event that both crossterm and ratzilla events can convert to.
#[derive(Debug, Clone)]
pub struct AppKeyEvent {
    pub code: AppKeyCode,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Abstract key code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppKeyCode {
    Char(char),
    Backspace,
    Enter,
    Left,
    Right,
    Up,
    Down,
    Tab,
    BackTab,
    Delete,
    Home,
    End,
    Esc,
    Other,
}

impl AppKeyEvent {
    pub fn is_ctrl_c(&self) -> bool {
        self.ctrl && self.code == AppKeyCode::Char('c')
    }
}
