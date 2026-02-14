use chrono::{DateTime, Utc};
use tui_textarea::{CursorMove, Input, Key, TextArea};

use crate::key_event::{AppKeyCode, AppKeyEvent};
use successlib::Goal;

// ── Single-line TextArea helpers ────────────────────────────────────────

pub fn single_line_textarea_from_string(value: String) -> TextArea<'static> {
    let mut textarea = TextArea::from([value]);
    textarea.move_cursor(CursorMove::End);
    textarea
}

pub fn single_line_textarea_value(textarea: &TextArea<'_>) -> String {
    textarea.lines().join("")
}

pub fn clear_single_line_textarea(textarea: &mut TextArea<'static>) {
    *textarea = TextArea::default();
}

/// Returns true if the key was consumed.
pub fn handle_single_line_textarea_key(
    textarea: &mut TextArea<'static>,
    key: &AppKeyEvent,
) -> bool {
    let Some(input) = app_key_to_textarea_input(key, false) else {
        return false;
    };

    let consumed = textarea.input(input);
    consumed
        || matches!(
            key.code,
            AppKeyCode::Left | AppKeyCode::Right | AppKeyCode::Home | AppKeyCode::End
        )
}

pub fn app_key_to_textarea_input(key: &AppKeyEvent, multiline: bool) -> Option<Input> {
    if key.ctrl {
        match key.code {
            AppKeyCode::Backspace => {
                return Some(Input {
                    key: Key::Backspace,
                    ctrl: false,
                    alt: true,
                    shift: key.shift,
                });
            }
            AppKeyCode::Delete => {
                return Some(Input {
                    key: Key::Delete,
                    ctrl: false,
                    alt: true,
                    shift: key.shift,
                });
            }
            _ => {}
        }
    }

    let mapped = match key.code {
        AppKeyCode::Char(c) => Key::Char(c),
        AppKeyCode::Backspace => Key::Backspace,
        AppKeyCode::Delete => Key::Delete,
        AppKeyCode::Left => Key::Left,
        AppKeyCode::Right => Key::Right,
        AppKeyCode::Home => Key::Home,
        AppKeyCode::End => Key::End,
        AppKeyCode::Up if multiline => Key::Up,
        AppKeyCode::Down if multiline => Key::Down,
        AppKeyCode::Enter if multiline => Key::Enter,
        AppKeyCode::Tab if multiline => Key::Tab,
        _ => return None,
    };

    Some(Input {
        key: mapped,
        ctrl: key.ctrl,
        alt: key.alt,
        shift: key.shift,
    })
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

#[derive(Debug, Default)]
pub struct FormState {
    pub current_field: FormField,
    pub goal_name: TextArea<'static>,
    pub quantity_name: TextArea<'static>,
    pub commands: TextArea<'static>,
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
