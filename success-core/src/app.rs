use chrono::{Local, NaiveDate};

use crate::handlers::*;
use crate::key_event::AppKeyEvent;
use crate::notes::refresh_notes_for_selection;
use crate::types::*;
use crate::ui::build_view_items;
use successlib::{Goal, SessionView};
use tui_textarea::TextArea;

/// Central application state, generic over the storage backend.
pub struct AppState {
    pub archive_path: String,
    pub goals: Vec<Goal>,
    pub nodes: Vec<SessionView>,
    pub current_day: NaiveDate,
    pub selected: usize,
    pub mode: Mode,
    pub search_input: TextArea<'static>,
    pub search_selected: usize,
    pub duration_input: TextArea<'static>,
    pub quantity_input: TextArea<'static>,
    pub timer: Option<TimerState>,
    pub pending_session: Option<PendingSession>,
    pub notes_textarea: TextArea<'static>,
    pub focused_block: FocusedBlock,
    pub form_state: Option<FormState>,
}

impl AppState {
    pub fn new(archive_path: String) -> Self {
        let today = Local::now().date_naive();
        let goals = successlib::list_goals(archive_path.clone(), None).unwrap_or_default();
        let nodes = successlib::list_day_sessions(
            archive_path.clone(),
            today.format("%Y-%m-%d").to_string(),
        )
        .unwrap_or_default();
        let mut state = Self {
            archive_path,
            goals,
            nodes,
            current_day: today,
            selected: 0,
            mode: Mode::View,
            search_input: TextArea::default(),
            search_selected: 0,
            duration_input: TextArea::default(),
            quantity_input: TextArea::default(),
            timer: None,
            pending_session: None,
            notes_textarea: TextArea::default(),
            focused_block: FocusedBlock::SessionsList,
            form_state: None,
        };
        state.selected = build_view_items(&state, 20).len().saturating_sub(1);
        refresh_notes_for_selection(&mut state);
        state
    }

    /// Dispatch a key event to the appropriate handler.
    /// Returns true if the app should quit (only for CLI).
    pub fn handle_key(&mut self, key: AppKeyEvent) -> bool {
        if key.is_ctrl_c() {
            return true;
        }
        match self.mode {
            Mode::View => handle_view_key(self, &key),
            Mode::AddSession | Mode::AddReward => handle_search_key(self, &key),
            Mode::GoalForm => handle_form_key(self, &key),
            Mode::QuantityDoneInput { .. } => handle_quantity_done_key(self, &key),
            Mode::DurationInput { .. } => handle_duration_key(self, &key),
            Mode::Timer => handle_timer_key(self, &key),
            Mode::NotesEdit => handle_notes_key(self, &key),
        }
        false
    }

    /// Tick the timer (call on every frame / poll cycle).
    pub fn tick(&mut self) {
        crate::timer::tick_timer(self);
    }
}
