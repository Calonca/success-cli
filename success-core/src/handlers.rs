use chrono::{Duration as ChronoDuration, Local};

use crate::app::AppState;
use crate::key_event::{AppKeyCode, AppKeyEvent};
use crate::notes::{refresh_notes_for_selection, save_notes_for_selection};
use crate::timer::{finalize_session, start_timer};
use crate::types::*;
use crate::ui::{build_view_items, ViewItemKind};
use crate::utils::{
    format_duration_suggestion, parse_commands_input, parse_duration, parse_optional_u32,
    selected_goal_id,
};

pub fn handle_view_key(state: &mut AppState, key: &AppKeyEvent) {
    match key.code {
        AppKeyCode::Char('q') => {} // Quit is handled by the caller
        AppKeyCode::Up | AppKeyCode::Char('k') => {
            let prev = state.selected;
            state.selected = state.selected.saturating_sub(1);
            if state.selected != prev {
                refresh_notes_for_selection(state);
            }
        }
        AppKeyCode::Left | AppKeyCode::Char('h') => {
            shift_day(state, -1);
        }
        AppKeyCode::Right | AppKeyCode::Char('l') => {
            shift_day(state, 1);
        }
        AppKeyCode::Down | AppKeyCode::Char('j') => {
            let max_idx = build_view_items(state, 20).len().saturating_sub(1);
            let prev = state.selected;
            state.selected = state.selected.min(max_idx).saturating_add(1).min(max_idx);
            if state.selected != prev {
                refresh_notes_for_selection(state);
            }
        }
        AppKeyCode::Char('e') => {
            if selected_goal_id(state).is_some() {
                refresh_notes_for_selection(state);
                state.mode = Mode::NotesEdit;
                state.focused_block = FocusedBlock::Notes;
            }
        }
        AppKeyCode::Enter => {
            let items = build_view_items(state, 20);
            let Some(item) = items.get(state.selected) else {
                return;
            };
            match item.kind {
                ViewItemKind::AddSession => {
                    if state.timer.is_some() {
                        return;
                    }
                    state.mode = Mode::AddSession;
                    state.search_input.clear();
                    state.search_selected = 0;
                }
                ViewItemKind::AddReward => {
                    if state.timer.is_some() {
                        return;
                    }
                    state.mode = Mode::AddReward;
                    state.search_input.clear();
                    state.search_selected = 0;
                }
                ViewItemKind::RunningTimer => {}
                ViewItemKind::Existing(_, _) => {}
            }
        }
        _ => {}
    }
}

pub fn shift_day(state: &mut AppState, delta: i64) {
    if delta == 0 {
        return;
    }
    let today = Local::now().date_naive();
    let Some(new_day) = state
        .current_day
        .checked_add_signed(ChronoDuration::days(delta))
    else {
        return;
    };
    if new_day > today {
        return;
    }
    state.current_day = new_day;
    state.nodes = successlib::list_day_sessions(
        state.archive_path.clone(),
        new_day.format("%Y-%m-%d").to_string(),
    )
    .unwrap_or_default();
    state.selected = build_view_items(state, 20).len().saturating_sub(1);
    refresh_notes_for_selection(state);
}

pub fn handle_search_key(state: &mut AppState, key: &AppKeyEvent) {
    if state.search_input.handle_key(key) {
        state.search_selected = 0;
        return;
    }
    match key.code {
        AppKeyCode::Esc => {
            state.mode = Mode::View;
            state.search_input.clear();
            state.search_selected = 0;
        }
        AppKeyCode::Enter => {
            let results = search_results(state);
            if let Some((_, result)) = results.get(state.search_selected) {
                state.search_input.clear();
                state.search_selected = 0;
                match result {
                    SearchResult::Create { name, is_reward } => {
                        state.form_state = Some(FormState {
                            current_field: FormField::GoalName,
                            goal_name: TextInput::from_string(name.clone()),
                            quantity_name: TextInput::default(),
                            commands: TextInput::default(),
                            is_reward: *is_reward,
                        });
                        state.mode = Mode::GoalForm;
                    }
                    SearchResult::Existing(goal) => {
                        let mut suggestion = None;
                        let recent = successlib::list_sessions_between_dates(
                            state.archive_path.clone(),
                            None,
                            None,
                        )
                        .unwrap_or_default();
                        if let Some(last) = recent
                            .iter()
                            .filter(|s| s.goal_id == goal.id)
                            .max_by_key(|s| s.start_at)
                        {
                            let duration_mins = (last.end_at - last.start_at) / 60;
                            suggestion = Some(format_duration_suggestion(duration_mins));
                        }
                        let suggestion = suggestion.unwrap_or_else(|| "25m".to_string());
                        state.duration_input = TextInput::from_string(suggestion);
                        state.mode = Mode::DurationInput {
                            is_reward: matches!(state.mode, Mode::AddReward),
                            goal_name: goal.name.clone(),
                            goal_id: goal.id,
                        };
                    }
                }
            }
        }
        AppKeyCode::Up => {
            if state.search_selected > 0 {
                state.search_selected -= 1;
            }
        }
        AppKeyCode::Down => {
            let len = search_results(state).len();
            if len > 0 {
                state.search_selected = (state.search_selected + 1).min(len - 1);
            }
        }
        _ => {}
    }
}

pub fn handle_form_key(state: &mut AppState, key: &AppKeyEvent) {
    let Some(form) = state.form_state.as_mut() else {
        state.mode = Mode::View;
        return;
    };

    let field = match form.current_field {
        FormField::GoalName => &mut form.goal_name,
        FormField::Quantity => &mut form.quantity_name,
        FormField::Commands => &mut form.commands,
    };

    if field.handle_key(key) {
        return;
    }

    match key.code {
        AppKeyCode::Esc => {
            state.form_state = None;
            state.mode = Mode::View;
        }
        AppKeyCode::Up | AppKeyCode::BackTab => {
            form.current_field = match form.current_field {
                FormField::GoalName => FormField::Commands,
                FormField::Quantity => FormField::GoalName,
                FormField::Commands => FormField::Quantity,
            };
        }
        AppKeyCode::Down | AppKeyCode::Tab => {
            form.current_field = match form.current_field {
                FormField::GoalName => FormField::Quantity,
                FormField::Quantity => FormField::Commands,
                FormField::Commands => FormField::GoalName,
            };
        }
        AppKeyCode::Enter => {
            let name = form.goal_name.value.trim().to_string();
            if name.is_empty() {
                return;
            }

            let commands = parse_commands_input(&form.commands.value);
            let quantity_name = if form.quantity_name.value.trim().is_empty() {
                None
            } else {
                Some(form.quantity_name.value.trim().to_string())
            };
            let is_reward = form.is_reward;

            let created = successlib::add_goal(
                state.archive_path.clone(),
                name.clone(),
                is_reward,
                commands,
                quantity_name,
            )
            .expect("Failed to add goal");
            state.goals.push(created.clone());

            state.form_state = None;
            state.duration_input = TextInput::from_string("25m".to_string());
            state.mode = Mode::DurationInput {
                is_reward,
                goal_name: created.name.clone(),
                goal_id: created.id,
            };
        }
        _ => {}
    }
}

pub fn handle_duration_key(state: &mut AppState, key: &AppKeyEvent) {
    if state.duration_input.handle_key(key) {
        return;
    }
    match key.code {
        AppKeyCode::Esc => {
            state.duration_input.clear();
            state.mode = Mode::View;
        }
        AppKeyCode::Enter => {
            let (is_reward, goal_name, goal_id) = match &state.mode {
                Mode::DurationInput {
                    is_reward,
                    goal_name,
                    goal_id,
                } => (*is_reward, goal_name.clone(), *goal_id),
                _ => return,
            };
            let secs = parse_duration(&state.duration_input.value).unwrap_or(25 * 60);
            start_timer(state, goal_name, goal_id, secs as u32, is_reward);
        }
        _ => {}
    }
}

pub fn handle_quantity_done_key(state: &mut AppState, key: &AppKeyEvent) {
    if !matches!(state.mode, Mode::QuantityDoneInput { .. }) {
        return;
    }

    if state.quantity_input.handle_key(key) {
        return;
    }

    match key.code {
        AppKeyCode::Esc => {
            state.quantity_input.clear();
            if let Some(pending) = state.pending_session.take() {
                finalize_session(state, pending, None);
            } else {
                state.mode = Mode::View;
            }
        }
        AppKeyCode::Enter => {
            let qty = parse_optional_u32(&state.quantity_input.value);
            if let Some(pending) = state.pending_session.take() {
                finalize_session(state, pending, qty);
            }
            state.quantity_input.clear();
        }
        _ => {}
    }
}

pub fn handle_timer_key(state: &mut AppState, key: &AppKeyEvent) {
    handle_view_key(state, key);
}

pub fn handle_notes_key(state: &mut AppState, key: &AppKeyEvent) {
    if key.ctrl {
        match key.code {
            AppKeyCode::Left => crate::notes::move_notes_cursor_word_left(state),
            AppKeyCode::Right => crate::notes::move_notes_cursor_word_right(state),
            _ => {}
        }
        return;
    }

    match key.code {
        AppKeyCode::Esc => {
            save_notes_for_selection(state);
            state.mode = Mode::View;
            state.focused_block = FocusedBlock::SessionsList;
        }
        AppKeyCode::Backspace => {
            if state.notes_cursor > 0 {
                let prev_len = state
                    .notes
                    .get(..state.notes_cursor)
                    .and_then(|s| s.chars().last())
                    .map(|c| c.len_utf8())
                    .unwrap_or(1);
                let start = state.notes_cursor - prev_len;
                state.notes.replace_range(start..state.notes_cursor, "");
                state.notes_cursor = start;
                save_notes_for_selection(state);
            }
        }
        AppKeyCode::Enter => {
            let insert_at = state.notes_cursor;
            state.notes.insert(insert_at, '\n');
            state.notes_cursor += 1;
            save_notes_for_selection(state);
        }
        AppKeyCode::Tab => {
            let insert_at = state.notes_cursor;
            state.notes.insert_str(insert_at, "    ");
            state.notes_cursor += 4;
            save_notes_for_selection(state);
        }
        AppKeyCode::Char(c) => {
            if !key.ctrl {
                let insert_at = state.notes_cursor;
                state.notes.insert(insert_at, c);
                state.notes_cursor += c.len_utf8();
                save_notes_for_selection(state);
            }
        }
        AppKeyCode::Left => {
            crate::notes::move_notes_cursor_left(state);
        }
        AppKeyCode::Right => {
            crate::notes::move_notes_cursor_right(state);
        }
        AppKeyCode::Up => {
            crate::notes::move_notes_cursor_vert(state, -1);
        }
        AppKeyCode::Down => {
            crate::notes::move_notes_cursor_vert(state, 1);
        }
        _ => {}
    }
}

pub fn search_results(state: &AppState) -> Vec<(String, SearchResult)> {
    let q = state.search_input.value.trim();
    let is_reward = matches!(state.mode, Mode::AddReward);

    let goals = successlib::search_goals(
        state.archive_path.clone(),
        q.to_string(),
        Some(is_reward),
        None,
        Some(true),
    )
    .unwrap_or_default();

    let mut results: Vec<(String, SearchResult)> = goals
        .into_iter()
        .map(|g| {
            (
                format!("{} (id {})", g.name, g.id),
                SearchResult::Existing(g),
            )
        })
        .collect();

    let create_label = format!("Create: {q}");
    results.push((
        create_label,
        SearchResult::Create {
            name: if q.is_empty() {
                if is_reward {
                    "New reward".to_string()
                } else {
                    "New goal".to_string()
                }
            } else {
                q.to_string()
            },
            is_reward,
        },
    ));

    results
}
