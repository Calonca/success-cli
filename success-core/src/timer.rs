use chrono::{Local, Utc};

use crate::app::AppState;
use crate::notes::{refresh_notes_for_selection, save_notes_for_selection};
use crate::types::*;
use crate::ui::build_view_items;
use crate::utils::goal_quantity_name;

pub fn tick_timer(state: &mut AppState) {
    if let Some(timer) = state.timer.as_mut() {
        let now_utc = Utc::now();
        let elapsed_seconds = (now_utc - timer.started_at).num_seconds();

        if elapsed_seconds >= 0 {
            let elapsed = elapsed_seconds as u64;
            if elapsed >= timer.total {
                timer.remaining = 0;
            } else {
                timer.remaining = timer.total - elapsed;
            }

            if timer.remaining == 0 {
                finish_timer(state);
            }
        }
    }
}

pub fn finish_timer(state: &mut AppState) {
    if let Some(timer) = state.timer.take() {
        if matches!(state.mode, Mode::NotesEdit) {
            save_notes_for_selection(state);
        }

        let pending = PendingSession {
            label: timer.label.clone(),
            goal_id: timer.goal_id,
            total: timer.total,
            is_reward: timer.is_reward,
            started_at: timer.started_at,
        };

        let quantity_name = goal_quantity_name(state, timer.goal_id);
        let needs_quantity = quantity_name.is_some();

        if needs_quantity {
            state.pending_session = Some(pending);
            state.quantity_input.clear();
            state.mode = Mode::QuantityDoneInput {
                goal_name: timer.label,
                quantity_name,
            };
            state.focused_block = FocusedBlock::SessionsList;
        } else {
            finalize_session(state, pending, None);
        }
    }
}

pub fn start_timer(
    state: &mut AppState,
    goal_name: String,
    goal_id: u64,
    seconds: u32,
    is_reward: bool,
) {
    if state.timer.is_some() {
        return;
    }

    let today = Local::now().date_naive();
    if state.current_day != today {
        state.current_day = today;
        state.nodes = successlib::list_day_sessions(state.archive_path.clone(), today.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        state.selected = build_view_items(state, 20).len().saturating_sub(1);
        refresh_notes_for_selection(state);
    }

    let started_at = Utc::now();

    // Append session start header to notes
    let mut note =
        successlib::get_note(state.archive_path.clone(), goal_id).unwrap_or_default();
    let start_local = started_at.with_timezone(&Local);
    let start_stamp = start_local.format("%Y-%m-%d %H:%M");
    note.push_str(&format!("---\n{start_stamp}\n"));
    let _ = successlib::edit_note(state.archive_path.clone(), goal_id, note);

    state.timer = Some(TimerState {
        label: goal_name,
        goal_id,
        remaining: seconds as u64,
        total: seconds as u64,
        is_reward,
        started_at,
    });
    state.selected = build_view_items(state, 20).len().saturating_sub(1);
    refresh_notes_for_selection(state);
    state.mode = Mode::Timer;
}

pub fn finalize_session(state: &mut AppState, pending: PendingSession, quantity: Option<u32>) {
    if matches!(state.mode, Mode::NotesEdit) {
        save_notes_for_selection(state);
    }

    state.mode = Mode::View;
    state.focused_block = FocusedBlock::SessionsList;
    let duration_secs = pending.total.min(u32::MAX as u64) as u32;
    let created = successlib::add_session(
        state.archive_path.clone(),
        pending.goal_id,
        pending.label.clone(),
        pending.started_at.timestamp(),
        duration_secs,
        pending.is_reward,
        quantity,
    )
    .expect("Failed to add session");

    let timer_day = chrono::DateTime::from_timestamp(created.start_at, 0)
        .map(|dt| dt.with_timezone(&Local).date_naive())
        .unwrap_or_else(|| Local::now().date_naive());
    if state.current_day == timer_day {
        state.nodes = successlib::list_day_sessions(state.archive_path.clone(), timer_day.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let items = build_view_items(state, 20);
        state.selected = items.len().saturating_sub(1);
        refresh_notes_for_selection(state);
    }
}
