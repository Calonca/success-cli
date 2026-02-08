use std::{cell::RefCell, rc::Rc};

use chrono::{NaiveDate, TimeZone, Utc};
use ratzilla::{
    backend::webgl2::FontAtlasConfig,
    backend::webgl2::WebGl2BackendOptions,
    event::{KeyCode, KeyEvent},
    WebGl2Backend, WebRenderer,
};
use success_core::app::AppState;
use success_core::key_event::{AppKeyCode, AppKeyEvent};
use success_core::notes::refresh_notes_for_selection;
use success_core::ui;
use successlib::Goal;

// ── Key event conversion ─────────────────────────────────────────────────

fn convert_key(key: &KeyEvent) -> AppKeyEvent {
    let code = match key.code {
        KeyCode::Char(c) => AppKeyCode::Char(c),
        KeyCode::Backspace => AppKeyCode::Backspace,
        KeyCode::Enter => AppKeyCode::Enter,
        KeyCode::Left => AppKeyCode::Left,
        KeyCode::Right => AppKeyCode::Right,
        KeyCode::Up => AppKeyCode::Up,
        KeyCode::Down => AppKeyCode::Down,
        KeyCode::Tab => {
            if key.shift {
                AppKeyCode::BackTab
            } else {
                AppKeyCode::Tab
            }
        }
        KeyCode::Delete => AppKeyCode::Delete,
        KeyCode::Home => AppKeyCode::Home,
        KeyCode::End => AppKeyCode::End,
        KeyCode::Esc => AppKeyCode::Esc,
        _ => AppKeyCode::Other,
    };
    AppKeyEvent {
        code,
        ctrl: key.ctrl,
        alt: key.alt,
        shift: key.shift,
    }
}

// ── Seed data (only when storage is empty) ───────────────────────────────

fn add_seed_session(
    archive: &str,
    goal: &Goal,
    start: chrono::DateTime<Utc>,
    duration_mins: u32,
    is_reward: bool,
    quantity: Option<u32>,
) {
    let _ = successlib::add_session(
        archive.to_string(),
        goal.id,
        goal.name.clone(),
        start.timestamp(),
        duration_mins.saturating_mul(60),
        is_reward,
        quantity,
    );
}

fn seed_sessions_for_day(
    archive: &str,
    day: NaiveDate,
    learn: &Goal,
    exercise: &Goal,
    read: &Goal,
    movie: &Goal,
    clean: &Goal,
    is_prev: bool,
) {
    if is_prev {
        let start1 = Utc.from_utc_datetime(&day.and_hms_opt(8, 30, 0).unwrap());
        let start2 = Utc.from_utc_datetime(&day.and_hms_opt(9, 10, 0).unwrap());
        let start3 = Utc.from_utc_datetime(&day.and_hms_opt(11, 0, 0).unwrap());
        let start4 = Utc.from_utc_datetime(&day.and_hms_opt(12, 15, 0).unwrap());

        add_seed_session(archive, read, start1, 30, false, Some(10));
        add_seed_session(archive, movie, start2, 20, true, None);
        add_seed_session(archive, clean, start3, 60, false, Some(1));
        add_seed_session(archive, movie, start4, 30, true, None);
        return;
    }

    let start1 = Utc.from_utc_datetime(&day.and_hms_opt(9, 0, 0).unwrap());
    let start2 = Utc.from_utc_datetime(&day.and_hms_opt(10, 15, 0).unwrap());
    let start3 = Utc.from_utc_datetime(&day.and_hms_opt(14, 0, 0).unwrap());
    let start4 = Utc.from_utc_datetime(&day.and_hms_opt(15, 10, 0).unwrap());

    add_seed_session(archive, learn, start1, 60, false, Some(2));
    add_seed_session(archive, movie, start2, 30, true, None);
    add_seed_session(archive, exercise, start3, 60, false, Some(3000));
    add_seed_session(archive, movie, start4, 30, true, None);
}

fn seed_if_empty(state: &mut AppState) {
    if !state.goals.is_empty() {
        return;
    }

    let archive = state.archive_path.clone();

    let learn = successlib::add_goal(
        archive.clone(),
        "Learn Rust".to_string(),
        false,
        vec![
            "code .".to_string(),
            "open https://doc.rust-lang.org".to_string(),
        ],
        Some("chapters".to_string()),
    )
    .expect("Failed to add seed goal");

    let exercise = successlib::add_goal(
        archive.clone(),
        "Exercise".to_string(),
        false,
        vec![],
        Some("steps".to_string()),
    )
    .expect("Failed to add seed goal");

    let read = successlib::add_goal(
        archive.clone(),
        "Read a book".to_string(),
        false,
        vec![],
        Some("pages".to_string()),
    )
    .expect("Failed to add seed goal");

    let movie = successlib::add_goal(
        archive.clone(),
        "Watch a movie".to_string(),
        true,
        vec![],
        None,
    )
    .expect("Failed to add seed goal");

    let clean = successlib::add_goal(
        archive.clone(),
        "Clean the house".to_string(),
        false,
        vec![],
        Some("rooms".to_string()),
    )
    .expect("Failed to add seed goal");

    let _ = successlib::edit_note(
        archive.clone(),
        learn.id,
        "Follow chapters 1-3 and take notes on ownership and borrowing.\n".to_string(),
    );
    let _ = successlib::edit_note(
        archive.clone(),
        exercise.id,
        "Warmup + 30 minutes of cardio.\n".to_string(),
    );
    let _ = successlib::edit_note(
        archive.clone(),
        read.id,
        "Read 10 pages and summarize the key ideas.\n".to_string(),
    );
    let _ = successlib::edit_note(
        archive.clone(),
        movie.id,
        "Reward: pick a feel-good movie.\n".to_string(),
    );
    let _ = successlib::edit_note(
        archive.clone(),
        clean.id,
        "Focus on 1 room and reset the space.\n".to_string(),
    );

    let today = state.current_day;
    seed_sessions_for_day(&archive, today, &learn, &exercise, &read, &movie, &clean, false);
    if let Some(prev_day) = today.pred_opt() {
        seed_sessions_for_day(&archive, prev_day, &learn, &exercise, &read, &movie, &clean, true);
    }

    state.goals = successlib::list_goals(archive.clone(), None).unwrap_or_default();
    state.nodes = successlib::list_day_sessions(
        archive,
        today.format("%Y-%m-%d").to_string(),
    )
    .unwrap_or_default();
    state.selected = ui::build_view_items(state, 20).len().saturating_sub(1);
    refresh_notes_for_selection(state);
}

// ── Main entry point ─────────────────────────────────────────────────────

fn main() {
    console_error_panic_hook::set_once();

    let mut app_state = AppState::new("success".to_string());
    seed_if_empty(&mut app_state);
    let state = Rc::new(RefCell::new(app_state));

    let backend = WebGl2Backend::new_with_options(
        WebGl2BackendOptions::new()
            .font_atlas_config(FontAtlasConfig::dynamic(&["JetBrains Mono"], 16.0)),
    )
    .expect("Failed to create WebGl2Backend");
    let terminal = ratzilla::ratatui::Terminal::new(backend).expect("Failed to create terminal");

    let state_key = Rc::clone(&state);
    terminal.on_key_event(move |key| {
        let mut s = state_key.borrow_mut();
        let app_key = convert_key(&key);
        s.handle_key(app_key);
    });

    let state_draw = Rc::clone(&state);
    terminal.draw_web(move |f| {
        let mut s = state_draw.borrow_mut();
        s.tick();
        let header = "Work on goals to receive rewards — Web demo — Data not persisted. Download full version: https://github.com/Calonca/success-cli".to_string();
        ui::ui(f, &s, &header);
    });
}
