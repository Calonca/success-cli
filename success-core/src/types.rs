use chrono::{DateTime, Utc};

use crate::key_event::{AppKeyCode, AppKeyEvent};
use successlib::Goal;

// ── TextInput ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
}

impl TextInput {
    pub fn new(value: String) -> Self {
        let len = value.chars().count();
        Self { value, cursor: len }
    }

    pub fn from_string(value: String) -> Self {
        Self::new(value)
    }

    /// Returns true if the key was consumed.
    pub fn handle_key(&mut self, key: &AppKeyEvent) -> bool {
        match key.code {
            AppKeyCode::Char(c) if !key.ctrl && !key.alt => {
                self.insert_char(c);
                true
            }
            AppKeyCode::Backspace => {
                self.delete_char_back();
                true
            }
            AppKeyCode::Delete => {
                self.delete_char_forward();
                true
            }
            AppKeyCode::Left => {
                if key.ctrl {
                    self.move_word_left();
                } else {
                    self.move_left();
                }
                true
            }
            AppKeyCode::Right => {
                if key.ctrl {
                    self.move_word_right();
                } else {
                    self.move_right();
                }
                true
            }
            AppKeyCode::Home => {
                self.cursor = 0;
                true
            }
            AppKeyCode::End => {
                self.cursor = self.value.chars().count();
                true
            }
            _ => false,
        }
    }

    pub fn insert_char(&mut self, c: char) {
        if self.cursor >= self.value.chars().count() {
            self.value.push(c);
            self.cursor += 1;
        } else {
            let mut result = String::new();
            for (i, ch) in self.value.chars().enumerate() {
                if i == self.cursor {
                    result.push(c);
                }
                result.push(ch);
            }
            self.value = result;
            self.cursor += 1;
        }
    }

    pub fn delete_char_back(&mut self) {
        if self.cursor > 0 {
            let mut result = String::new();
            for (i, ch) in self.value.chars().enumerate() {
                if i != self.cursor - 1 {
                    result.push(ch);
                }
            }
            self.value = result;
            self.cursor -= 1;
        }
    }

    pub fn delete_char_forward(&mut self) {
        if self.cursor < self.value.chars().count() {
            let mut result = String::new();
            for (i, ch) in self.value.chars().enumerate() {
                if i != self.cursor {
                    result.push(ch);
                }
            }
            self.value = result;
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.value.chars().count() {
            self.cursor += 1;
        }
    }

    pub fn move_word_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.value.chars().collect();
        let mut idx = self.cursor;
        while idx > 0 && idx <= chars.len() && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        while idx > 0 && idx <= chars.len() && !chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        self.cursor = idx;
    }

    pub fn move_word_right(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let len = chars.len();
        if self.cursor >= len {
            return;
        }
        let mut idx = self.cursor;
        while idx < len && !chars[idx].is_whitespace() {
            idx += 1;
        }
        while idx < len && chars[idx].is_whitespace() {
            idx += 1;
        }
        self.cursor = idx;
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }
}

// ── Enums ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Mode {
    View,
    AddSession,
    AddReward,
    GoalForm,
    QuantityDoneInput {
        goal_name: String,
        quantity_name: Option<String>,
    },
    DurationInput {
        is_reward: bool,
        goal_name: String,
        goal_id: u64,
    },
    Timer,
    NotesEdit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormField {
    #[default]
    GoalName,
    Quantity,
    Commands,
}

#[derive(Debug, Clone, Default)]
pub struct FormState {
    pub current_field: FormField,
    pub goal_name: TextInput,
    pub quantity_name: TextInput,
    pub commands: TextInput,
    pub is_reward: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusedBlock {
    #[default]
    SessionsList,
    Notes,
}

#[derive(Debug, Clone)]
pub enum SearchResult {
    Existing(Goal),
    Create { name: String, is_reward: bool },
}

// ── Timer / Pending ──────────────────────────────────────────────────────

#[derive(Debug)]
pub struct TimerState {
    pub label: String,
    pub goal_id: u64,
    pub remaining: u64,
    pub total: u64,
    pub is_reward: bool,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PendingSession {
    pub label: String,
    pub goal_id: u64,
    pub total: u64,
    pub is_reward: bool,
    pub started_at: DateTime<Utc>,
}
