use libc::{kill, setsid, SIGTERM};
use std::fs;
use std::io::{self, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::Duration as ChronoDuration;
use chrono::{DateTime, Local, NaiveDate, Utc};
use crossterm::cursor::{MoveTo, SetCursorStyle};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};
use crossterm::{execute, terminal};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;
use serde::{Deserialize, Serialize};
use successlib::{
    add_goal, add_session, edit_note, edit_note_api, get_formatted_session_time_range, get_note,
    list_day_sessions, list_goals, list_sessions_between_dates, search_goals, Goal, Session,
    SessionKind,
};
#[cfg(target_os = "macos")]
const FILE_MANAGER_COMMAND: &str = "open";
#[cfg(target_os = "windows")]
const FILE_MANAGER_COMMAND: &str = "explorer";
#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
const FILE_MANAGER_COMMAND: &str = "xdg-open";

const DEFAULT_EDITOR: &str = "nvim";

fn render_timer_footer(f: &mut ratatui::Frame, area: ratatui::layout::Rect, timer: &TimerState) {
    let pct = if timer.total == 0 {
        0.0
    } else {
        1.0 - (timer.remaining as f32 / timer.total as f32)
    };
    let ratio = pct.clamp(0.0, 1.0) as f64;
    let block = Block::default().borders(Borders::ALL).title(format!(
        "{} timer",
        if timer.is_reward { "Reward" } else { "Session" }
    ));

    // Draw border first, then split inner area for text and the gauge bar
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    let para = Paragraph::new(Line::from(format!(
        "{} | Remaining: {}s | Total: {}s | Started: {}",
        timer.label,
        timer.remaining,
        timer.total,
        timer.started_at.with_timezone(&Local).format("%H:%M"),
    )));
    f.render_widget(para, chunks[0]);

    let gauge_label = format!(
        "{:.0}% • Remaining: {}s • Total: {}s • Started: {}",
        ratio * 100.0,
        timer.remaining,
        timer.total,
        timer.started_at.with_timezone(&Local).format("%H:%M"),
    );

    let gauge = Gauge::default()
        .ratio(ratio)
        .gauge_style(Style::default().fg(Color::Rgb(0, 130, 0)))
        .use_unicode(true)
        .label(gauge_label);
    f.render_widget(gauge, chunks[1]);
}

fn kind_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Goal => "session",
        SessionKind::Reward => "reward",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CliConfig {
    archive: Option<PathBuf>,
}

#[derive(Debug)]
enum Mode {
    View,
    AddSession,
    AddReward,
    CommandInput {
        goal_name: String,
        is_reward: bool,
        input: String,
    },
    DurationInput {
        is_reward: bool,
        goal_name: String,
        goal_id: u64,
    },
    Timer,
    NotesEdit,
}

fn format_mode(mode: &Mode) -> &'static str {
    match mode {
        Mode::View => "view",
        Mode::AddSession => "add-session",
        Mode::AddReward => "add-reward",
        Mode::CommandInput { .. } => "commands",
        Mode::DurationInput { .. } => "duration",
        Mode::Timer => "timer",
        Mode::NotesEdit => "notes-edit",
    }
}

#[derive(Debug)]
struct AppState {
    archive: PathBuf,
    goals: Vec<Goal>,
    nodes: Vec<Session>,
    current_day: NaiveDate,
    selected: usize,
    mode: Mode,
    search_input: String,
    search_selected: usize,
    duration_input: String,
    timer: Option<TimerState>,
    notes: String,
    notes_cursor: usize,
    status: Option<String>,
    needs_full_redraw: bool,
}

#[derive(Debug)]
struct TimerState {
    label: String,
    goal_id: u64,
    remaining: u64,
    total: u64,
    is_reward: bool,
    spawned: Vec<SpawnedCommand>,
    started_at: DateTime<Utc>,
}

#[derive(Debug)]
struct SpawnedCommand {
    command: String,
    child: Option<Child>,
    pgid: i32,
}

fn main() -> Result<()> {
    let archive = resolve_archive_interactive(None)?;
    let today = Local::now().date_naive();
    let mut state = AppState {
        archive: archive.clone(),
        goals: list_goals(&archive, None)?,
        nodes: list_day_sessions(&archive, today)?,
        current_day: today,
        selected: 0,
        mode: Mode::View,
        search_input: String::new(),
        search_selected: 0,
        duration_input: String::new(),
        timer: None,
        notes: String::new(),
        notes_cursor: 0,
        status: None,
        needs_full_redraw: false,
    };

    // Start focused on the most recent item (last in list) if any exist.
    state.selected = build_view_items(&state).len().saturating_sub(1);

    refresh_notes_for_selection(&mut state)?;

    persist_config(&archive).ok();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        terminal::EnterAlternateScreen,
        event::EnableMouseCapture,
        SetCursorStyle::SteadyBar
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, &mut state);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        terminal::LeaveAlternateScreen,
        event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Error: {err}");
    }
    Ok(())
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    state: &mut AppState,
) -> Result<()> {
    loop {
        if state.needs_full_redraw {
            terminal.clear()?;
            state.needs_full_redraw = false;
        }
        if state.timer.is_some() {
            tick_timer(state);
        }

        terminal.draw(|f| ui(f, state))?;

        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) => {
                    if handle_key(state, key)? {
                        break;
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    if let Some(timer) = state.timer.as_mut() {
        kill_spawned(&mut timer.spawned);
    }
    Ok(())
}

fn ui(f: &mut ratatui::Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(7),
        ])
        .split(f.size());

    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[1]);

    let mut header_line = format!(
        "Archive: {} (open with 'o') | Mode: {} ",
        state.archive.display(),
        format_mode(&state.mode),
    );
    if let Some(timer) = &state.timer {
        header_line.push_str(&format!(
            " | Timer: {} ({}s left)",
            timer.label, timer.remaining
        ));
    }
    if let Some(status) = &state.status {
        header_line.push_str(&format!(" | {status}"));
    }

    let header = Paragraph::new(header_line)
        .block(Block::default().borders(Borders::ALL).title("Success CLI"));
    f.render_widget(header, chunks[0]);

    let items = build_view_items(state);
    let list_items: Vec<ListItem> = items
        .iter()
        .map(|item| ListItem::new(Line::from(item.label.clone())))
        .collect();
    let title = format!("Sessions of {}", format_day_label(state.current_day));
    let list = List::new(list_items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let mut stateful = ratatui::widgets::ListState::default();
    if !items.is_empty() {
        stateful.select(Some(state.selected.min(items.len() - 1)));
    }
    f.render_stateful_widget(list, body_chunks[0], &mut stateful);

    let notes_title = if matches!(state.mode, Mode::NotesEdit) {
        "Notes (Esc to stop editing)"
    } else {
        "Notes (press 'e' to edit, 'E' for external editor)"
    };
    let notes_block = Block::default().borders(Borders::ALL).title(notes_title);

    if selected_goal_id(state).is_some() {
        let (cursor_line, cursor_col) = notes_cursor_line_col(state);
        let view_height = body_chunks[1].height.max(1) as usize;
        let desired_mid = view_height / 2;
        let offset = cursor_line.saturating_sub(desired_mid);
        let offset_u16 = offset.min(u16::MAX as usize) as u16;
        let notes_para = Paragraph::new(state.notes.clone())
            .block(notes_block)
            .scroll((offset_u16, 0));
        f.render_widget(notes_para, body_chunks[1]);

        if matches!(state.mode, Mode::NotesEdit) {
            let visible_line = cursor_line
                .saturating_sub(offset)
                .min(view_height.saturating_sub(1));
            let cursor_y = body_chunks[1].y + visible_line as u16;
            let cursor_x = body_chunks[1].x
                + cursor_col.min(body_chunks[1].width.saturating_sub(1) as usize) as u16;
            f.set_cursor(cursor_x + 1, cursor_y + 1);
        }
    } else {
        let notes_para = Paragraph::new("Select a task to view notes").block(notes_block);
        f.render_widget(notes_para, body_chunks[1]);
    }

    let mut footer_constraints = Vec::new();
    if state.timer.is_some() {
        // Give the timer footer enough height for text + the bar line inside a bordered block
        footer_constraints.push(Constraint::Length(4));
    }
    footer_constraints.push(Constraint::Min(4));

    let footer_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(footer_constraints)
        .split(chunks[2]);
    let mut footer_idx = 0;
    if let Some(timer) = &state.timer {
        render_timer_footer(f, footer_chunks[footer_idx], timer);
        footer_idx += 1;
    }
    let footer_area = footer_chunks[footer_idx];

    match state.mode {
        Mode::AddSession | Mode::AddReward => {
            let prompt = if matches!(state.mode, Mode::AddReward) {
                "Choose reward (type to search, Enter to pick, Esc to cancel)"
            } else {
                "Choose goal (type to search, Enter to pick, Esc to cancel)"
            };

            let search_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(footer_area);

            let input_line = format!("> {}", state.search_input);
            let input_para = Paragraph::new(input_line.clone());
            f.render_widget(input_para, search_layout[0]);

            let results = search_results(state);
            let list_items: Vec<ListItem> = results
                .iter()
                .map(|(label, _)| ListItem::new(Line::from(label.clone())))
                .collect();
            let mut list_state = ListState::default();
            if !results.is_empty() {
                list_state.select(Some(state.search_selected.min(results.len() - 1)));
            }
            let list = List::new(list_items)
                .block(Block::default().borders(Borders::ALL).title(prompt))
                .highlight_style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                );
            f.render_stateful_widget(list, search_layout[1], &mut list_state);

            // Place cursor at end of search input
            let cursor_x = search_layout[0].x + 2 + state.search_input.len() as u16;
            let cursor_y = search_layout[0].y;
            f.set_cursor(
                cursor_x.min(search_layout[0].x + search_layout[0].width.saturating_sub(1)),
                cursor_y,
            );
        }
        Mode::DurationInput { ref goal_name, .. } => {
            let title = format!("Duration for {goal_name} (e.g., 30m, 1h)");
            let block = Block::default().borders(Borders::ALL).title(title);
            let para = Paragraph::new(state.duration_input.clone()).block(block);
            f.render_widget(para, footer_area);
        }
        Mode::CommandInput {
            ref goal_name,
            ref input,
            ..
        } => {
            let title = format!(
                "Commands for {} (separate with ';', Enter to continue)",
                goal_name
            );
            let block = Block::default().borders(Borders::ALL).title(title);
            let para = Paragraph::new(input.clone()).block(block);
            f.render_widget(para, footer_area);
        }
        Mode::Timer => {
            let help = Paragraph::new(
                "Timer running • Up/Down/Left/Right navigate • 'e' edit notes • Finish before starting another",
            )
            .block(Block::default().borders(Borders::ALL));
            f.render_widget(help, footer_area);
        }
        Mode::View | Mode::NotesEdit => {
            let help = Paragraph::new(
                "Up/Down move • Left/Right change day • Enter to add/select • q to quit | Add goal/reward with Enter on [+] rows | 'e' to edit notes, Esc to exit notes",
            )
            .block(Block::default().borders(Borders::ALL));
            f.render_widget(help, footer_area);
        }
    }
}

#[derive(Debug, Clone)]
struct ViewItem {
    label: String,
    kind: ViewItemKind,
}

#[derive(Debug, Clone)]
enum SearchResult {
    Existing(Goal),
    Create { name: String, is_reward: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewItemKind {
    RunningTimer,
    Existing(SessionKind, usize),
    AddSession,
    AddReward,
}

fn build_view_items(state: &AppState) -> Vec<ViewItem> {
    let mut items = Vec::new();
    for (idx, n) in state.nodes.iter().enumerate() {
        let prefix = match n.kind {
            SessionKind::Goal => "[S]",
            SessionKind::Reward => "[R]",
        };
        let duration = (n.end_at - n.start_at).num_minutes();
        let times = get_formatted_session_time_range(n);
        items.push(ViewItem {
            label: format!("{prefix} {} ({duration}m) [{times}]", n.name),
            kind: ViewItemKind::Existing(n.kind, idx),
        });
    }
    if let Some(timer) = &state.timer {
        let started_local = timer.started_at.with_timezone(&Local).format("%H:%M");
        items.push(ViewItem {
            label: format!(
                "[*] Running: {} ({}s left) [started {started_local}]",
                timer.label, timer.remaining
            ),
            kind: ViewItemKind::RunningTimer,
        });
    }
    // Only offer add rows when no timer is running AND we are viewing today.
    if state.timer.is_none() && state.current_day == Local::now().date_naive() {
        if state
            .nodes
            .last()
            .map(|n| n.kind == SessionKind::Goal)
            .unwrap_or(false)
        {
            items.push(ViewItem {
                label: "[+] Receive reward".to_string(),
                kind: ViewItemKind::AddReward,
            });
        } else {
            items.push(ViewItem {
                label: "[+] Work on new goal".to_string(),
                kind: ViewItemKind::AddSession,
            });
        }
    }
    items
}

fn handle_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(key.code, KeyCode::Char('c')) {
            return Ok(true);
        }
    }
    match state.mode {
        Mode::View => handle_view_key(state, key),
        Mode::AddSession | Mode::AddReward => handle_search_key(state, key),
        Mode::CommandInput { .. } => handle_command_key(state, key),
        Mode::DurationInput { .. } => handle_duration_key(state, key),
        Mode::Timer => handle_timer_key(state, key),
        Mode::NotesEdit => handle_notes_key(state, key),
    }
}

fn handle_view_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Up | KeyCode::Char('k') => {
            let prev = state.selected;
            state.selected = state.selected.saturating_sub(1);
            if state.selected != prev {
                refresh_notes_for_selection(state)?;
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            shift_day(state, -1)?;
        }
        KeyCode::Right | KeyCode::Char('l') => {
            shift_day(state, 1)?;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let max_idx = build_view_items(state).len().saturating_sub(1);
            let prev = state.selected;
            state.selected = state.selected.min(max_idx).saturating_add(1).min(max_idx);
            if state.selected != prev {
                refresh_notes_for_selection(state)?;
            }
        }
        KeyCode::Char('e') => {
            if selected_goal_id(state).is_some() {
                refresh_notes_for_selection(state)?;
                state.mode = Mode::NotesEdit;
            }
        }
        KeyCode::Char('E') => {
            if selected_goal_id(state).is_some() {
                match open_notes_in_external_editor(state) {
                    Ok(_) => {
                        state.status = Some("Notes updated via external editor".to_string());
                    }
                    Err(err) => {
                        state.status = Some(format!("Failed to open editor: {err}"));
                    }
                }
            } else {
                state.status = Some("Select a goal before editing notes".to_string());
            }
        }
        KeyCode::Char('o') => match open_archive_in_file_manager(state) {
            Ok(_) => {
                state.status = Some("Opening archive in file manager".to_string());
            }
            Err(err) => {
                state.status = Some(format!("Failed to open archive: {err}"));
            }
        },
        KeyCode::Enter => {
            let items = build_view_items(state);
            let Some(item) = items.get(state.selected) else {
                return Ok(false);
            };
            match item.kind {
                ViewItemKind::AddSession => {
                    if state.timer.is_some() {
                        state.status =
                            Some("Finish the running session before starting another".to_string());
                        return Ok(false);
                    }
                    state.mode = Mode::AddSession;
                    state.search_input.clear();
                    state.search_selected = 0;
                }
                ViewItemKind::AddReward => {
                    if state.timer.is_some() {
                        state.status =
                            Some("Finish the running session before starting another".to_string());
                        return Ok(false);
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
    Ok(false)
}

fn shift_day(state: &mut AppState, delta: i64) -> Result<()> {
    if delta == 0 {
        return Ok(());
    }
    let today = Local::now().date_naive();
    let Some(new_day) = state
        .current_day
        .checked_add_signed(ChronoDuration::days(delta))
    else {
        state.status = Some("Day change out of range".to_string());
        return Ok(());
    };
    if new_day > today {
        state.status = Some("Cannot view future days".to_string());
        return Ok(());
    }

    state.current_day = new_day;
    state.nodes = list_day_sessions(&state.archive, new_day)?;
    state.selected = build_view_items(state).len().saturating_sub(1);
    refresh_notes_for_selection(state)?;
    state.status = Some(format!("Showing {}", format_day_label(new_day)));
    Ok(())
}

fn handle_search_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => {
            state.mode = Mode::View;
            state.search_input.clear();
            state.search_selected = 0;
        }
        KeyCode::Enter => {
            let results = search_results(state);
            if let Some((_, result)) = results.get(state.search_selected) {
                state.search_input.clear();
                state.search_selected = 0;
                match result {
                    SearchResult::Create { name, is_reward } => {
                        state.mode = Mode::CommandInput {
                            goal_name: name.clone(),
                            is_reward: *is_reward,
                            input: String::new(),
                        };
                    }
                    SearchResult::Existing(goal) => {
                        let mut suggestion = None;
                        if let Ok(recent) = list_sessions_between_dates(&state.archive, None, None)
                        {
                            if let Some(last) = recent
                                .iter()
                                .filter(|s| s.goal_id == goal.id)
                                .max_by_key(|s| s.start_at)
                            {
                                let duration_mins = (last.end_at - last.start_at).num_minutes();
                                suggestion = Some(format_duration_suggestion(duration_mins));
                            }
                        }
                        let suggestion = suggestion.unwrap_or_else(|| "25m".to_string());
                        state.duration_input = suggestion;
                        state.mode = Mode::DurationInput {
                            is_reward: matches!(state.mode, Mode::AddReward),
                            goal_name: goal.name.clone(),
                            goal_id: goal.id,
                        };
                    }
                }
            }
        }
        KeyCode::Backspace => {
            state.search_input.pop();
            let len = search_results(state).len();
            if len == 0 {
                state.search_selected = 0;
            } else {
                state.search_selected = state.search_selected.min(len.saturating_sub(1));
            }
        }
        KeyCode::Up => {
            if state.search_selected > 0 {
                state.search_selected -= 1;
            }
        }
        KeyCode::Down => {
            let len = search_results(state).len();
            if len > 0 {
                state.search_selected = (state.search_selected + 1).min(len - 1);
            }
        }
        KeyCode::Char(c) => {
            if key.modifiers != KeyModifiers::CONTROL {
                state.search_input.push(c);
                let len = search_results(state).len();
                state.search_selected = if len > 0 {
                    state.search_selected.min(len - 1)
                } else {
                    0
                };
            }
        }
        _ => {}
    }
    Ok(false)
}

fn handle_command_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    let Mode::CommandInput {
        ref goal_name,
        is_reward,
        ref mut input,
    } = state.mode
    else {
        return Ok(false);
    };

    match key.code {
        KeyCode::Esc => {
            input.clear();
            state.mode = Mode::View;
        }
        KeyCode::Enter => {
            let commands = parse_commands_input(input);
            let created = add_goal(&state.archive, goal_name, is_reward, commands)?;
            state.goals.push(created.clone());

            state.duration_input = "25m".to_string();
            let goal_name = created.name.clone();
            let goal_id = created.id;
            state.mode = Mode::DurationInput {
                is_reward: created.is_reward,
                goal_name,
                goal_id,
            };
        }
        KeyCode::Backspace => {
            input.pop();
        }
        KeyCode::Char(c) => {
            if key.modifiers != KeyModifiers::CONTROL {
                input.push(c);
            }
        }
        _ => {}
    }

    Ok(false)
}

fn handle_duration_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => {
            state.duration_input.clear();
            state.mode = Mode::View;
        }
        KeyCode::Enter => {
            let Mode::DurationInput {
                is_reward,
                ref goal_name,
                goal_id,
            } = state.mode
            else {
                return Ok(false);
            };
            let secs = parse_duration(&state.duration_input).unwrap_or(25 * 60);
            start_timer(state, goal_name.clone(), goal_id, secs as u32, is_reward)?;
        }
        KeyCode::Backspace => {
            state.duration_input.pop();
        }
        KeyCode::Char(c) => {
            if key.modifiers != KeyModifiers::CONTROL {
                state.duration_input.push(c);
            }
        }
        _ => {}
    }
    Ok(false)
}

fn handle_timer_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    // Allow all navigation and editing actions during timer, just not starting new sessions
    handle_view_key(state, key)
}

fn handle_notes_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Left => move_notes_cursor_word_left(state),
            KeyCode::Right => move_notes_cursor_word_right(state),
            _ => {}
        }
        return Ok(false);
    }

    match key.code {
        KeyCode::Esc => {
            save_notes_for_selection(state)?;
            state.mode = Mode::View;
        }
        KeyCode::Backspace => {
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
                save_notes_for_selection(state)?;
            }
        }
        KeyCode::Enter => {
            let insert_at = state.notes_cursor;
            state.notes.insert(insert_at, '\n');
            state.notes_cursor += 1;
            save_notes_for_selection(state)?;
        }
        KeyCode::Tab => {
            let insert_at = state.notes_cursor;
            state.notes.insert_str(insert_at, "    ");
            state.notes_cursor += 4;
            save_notes_for_selection(state)?;
        }
        KeyCode::Char(c) => {
            if key.modifiers != KeyModifiers::CONTROL {
                let insert_at = state.notes_cursor;
                state.notes.insert(insert_at, c);
                state.notes_cursor += c.len_utf8();
                save_notes_for_selection(state)?;
            }
        }
        KeyCode::Left => move_notes_cursor_left(state),
        KeyCode::Right => move_notes_cursor_right(state),
        KeyCode::Up => move_notes_cursor_vert(state, -1),
        KeyCode::Down => move_notes_cursor_vert(state, 1),
        _ => {}
    }
    Ok(false)
}

fn tick_timer(state: &mut AppState) {
    if let Some(timer) = state.timer.as_mut() {
        // Calculate elapsed time based on real-world clock time (DateTime)
        // instead of process time (Instant) so it works correctly when
        // the PC is suspended or the terminal is backgrounded
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

fn finish_timer(state: &mut AppState) {
    if let Some(mut timer) = state.timer.take() {
        // Only kill spawned apps when finishing a reward, not a regular session
        if timer.is_reward {
            kill_spawned(&mut timer.spawned);
        }
        state.mode = Mode::View;

        let duration_secs = timer.total.min(u32::MAX as u64) as u32;
        let created = match add_session(
            &state.archive,
            timer.goal_id,
            &timer.label,
            timer.started_at,
            duration_secs,
            timer.is_reward,
        ) {
            Ok(session) => session,
            Err(e) => {
                state.status = Some(format!("Failed to record session: {e}"));
                return;
            }
        };

        let timer_day = created.start_at.with_timezone(&Local).date_naive();
        if state.current_day == timer_day {
            match list_day_sessions(&state.archive, timer_day) {
                Ok(nodes) => {
                    state.nodes = nodes;
                    let items = build_view_items(state);
                    state.selected = items.len().saturating_sub(1);
                    if let Err(e) = refresh_notes_for_selection(state) {
                        eprintln!("Failed to load notes: {e}");
                    }
                }
                Err(e) => eprintln!("Failed to load day graph: {e}"),
            }
        }
        let kind = if timer.is_reward {
            SessionKind::Reward
        } else {
            SessionKind::Goal
        };
        state.status = Some(format!("Finished {}", kind_label(kind)));
    }
}

fn start_timer(
    state: &mut AppState,
    goal_name: String,
    goal_id: u64,
    seconds: u32,
    is_reward: bool,
) -> Result<()> {
    if state.timer.is_some() {
        state.status = Some("Finish the running session before starting another".to_string());
        return Ok(());
    }

    let today = Local::now().date_naive();
    if state.current_day != today {
        state.current_day = today;
        state.nodes = list_day_sessions(&state.archive, today)?;
        state.selected = build_view_items(state).len().saturating_sub(1);
        refresh_notes_for_selection(state)?;
    }

    let started_at = Utc::now();
    let commands = commands_for_goal(state, goal_id);
    let spawned = spawn_commands(&commands);
    state.timer = Some(TimerState {
        label: goal_name,
        goal_id,
        remaining: seconds as u64,
        total: seconds as u64,
        is_reward,
        spawned,
        started_at,
    });
    state.selected = build_view_items(state).len().saturating_sub(1);
    if let Err(e) = append_session_start_header(&state.archive, goal_id, started_at) {
        state.status = Some(format!("Failed to prepare notes: {e}"));
    }
    refresh_notes_for_selection(state)?;
    state.mode = Mode::Timer;
    state.status = Some(format!(
        "Running {}",
        if is_reward { "reward" } else { "session" }
    ));
    Ok(())
}

fn search_results(state: &AppState) -> Vec<(String, SearchResult)> {
    let q = state.search_input.trim();
    let is_reward = matches!(state.mode, Mode::AddReward);

    // Use the library function which handles fuzzy matching and sorting by recent
    let goals = search_goals(&state.archive, q, Some(is_reward), None, true).unwrap_or_default();

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

fn selected_goal_id(state: &AppState) -> Option<u64> {
    let items = build_view_items(state);
    match items.get(state.selected).map(|v| v.kind) {
        Some(ViewItemKind::RunningTimer) => state.timer.as_ref().map(|t| t.goal_id),
        Some(ViewItemKind::Existing(_, idx)) => state.nodes.get(idx).map(|n| n.goal_id),
        _ => state.timer.as_ref().map(|t| t.goal_id),
    }
}

fn append_session_start_header(
    archive: &Path,
    goal_id: u64,
    start_at: DateTime<Utc>,
) -> Result<()> {
    let mut note = get_note(archive, goal_id).unwrap_or_default();

    let start_local = start_at.with_timezone(&Local);
    let start_stamp = start_local.format("%Y-%m-%d %H:%M");
    note.push_str(&format!("---\n{start_stamp}\n"));

    edit_note_api(archive.to_string_lossy().to_string(), goal_id, note)
        .context("Failed to append session header")?;

    Ok(())
}

fn refresh_notes_for_selection(state: &mut AppState) -> Result<()> {
    if let Some(goal_id) = selected_goal_id(state) {
        state.notes = get_note(&state.archive, goal_id)?;
        state.notes_cursor = state.notes.len();
    } else {
        state.notes.clear();
        state.notes_cursor = 0;
    }
    Ok(())
}

fn save_notes_for_selection(state: &AppState) -> Result<()> {
    if let Some(goal_id) = selected_goal_id(state) {
        edit_note(&state.archive, goal_id, &state.notes)?;
    }
    Ok(())
}

fn notes_cursor_line_col(state: &AppState) -> (usize, usize) {
    let mut line = 0usize;
    let mut col = 0usize;
    let mut seen = 0usize;
    for (idx, ch) in state.notes.char_indices() {
        if idx >= state.notes_cursor {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
        seen = idx + ch.len_utf8();
    }
    // If cursor is at end and notes end with newline, cursor is start of next line
    if state.notes_cursor > seen {
        // beyond last processed char, adjust based on trailing segment
        let remaining = &state.notes[seen..state.notes_cursor];
        for ch in remaining.chars() {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
    }
    (line, col)
}

fn line_starts(notes: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (idx, ch) in notes.char_indices() {
        if ch == '\n' {
            starts.push(idx + ch.len_utf8());
        }
    }
    starts
}

fn move_notes_cursor_left(state: &mut AppState) {
    if state.notes_cursor == 0 {
        return;
    }
    if let Some((idx, _)) = state
        .notes
        .char_indices()
        .take_while(|(i, _)| *i < state.notes_cursor)
        .last()
    {
        state.notes_cursor = idx;
    } else {
        state.notes_cursor = 0;
    }
}

fn move_notes_cursor_right(state: &mut AppState) {
    let len = state.notes.len();
    if state.notes_cursor >= len {
        state.notes_cursor = len;
        return;
    }
    if let Some((idx, ch)) = state
        .notes
        .char_indices()
        .skip_while(|(i, _)| *i < state.notes_cursor)
        .next()
    {
        state.notes_cursor = idx + ch.len_utf8();
    } else {
        state.notes_cursor = len;
    }
}

fn move_notes_cursor_word_left(state: &mut AppState) {
    if state.notes_cursor == 0 {
        return;
    }
    let mut idx = state.notes_cursor;
    // Skip trailing whitespace to the left
    while idx > 0 {
        if let Some(ch) = state.notes[..idx].chars().rev().next() {
            if ch.is_whitespace() {
                idx = idx.saturating_sub(ch.len_utf8());
            } else {
                break;
            }
        } else {
            break;
        }
    }
    // Skip the word
    while idx > 0 {
        if let Some(ch) = state.notes[..idx].chars().rev().next() {
            if !ch.is_whitespace() {
                idx = idx.saturating_sub(ch.len_utf8());
            } else {
                break;
            }
        } else {
            break;
        }
    }
    state.notes_cursor = idx;
}

fn move_notes_cursor_word_right(state: &mut AppState) {
    let len = state.notes.len();
    let mut idx = state.notes_cursor;
    if idx >= len {
        return;
    }
    // Skip whitespace to the right
    while idx < len {
        if let Some(ch) = state.notes[idx..].chars().next() {
            if ch.is_whitespace() {
                idx += ch.len_utf8();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    // Skip the word
    while idx < len {
        if let Some(ch) = state.notes[idx..].chars().next() {
            if !ch.is_whitespace() {
                idx += ch.len_utf8();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    state.notes_cursor = idx;
}

fn move_notes_cursor_vert(state: &mut AppState, delta: isize) {
    let starts = line_starts(&state.notes);
    let (line, col) = notes_cursor_line_col(state);
    let new_line = line as isize + delta;
    if new_line < 0 || new_line as usize >= starts.len() {
        return;
    }
    let new_line = new_line as usize;
    let line_start = starts[new_line];
    let line_end = if new_line + 1 < starts.len() {
        // exclude the newline char
        starts[new_line + 1].saturating_sub(1)
    } else {
        state.notes.len()
    };
    let line_len = line_end.saturating_sub(line_start);
    let target_col = col.min(line_len);
    // Walk from line_start to target_col chars to find byte offset
    let mut byte = line_start;
    let mut remaining = target_col;
    for (idx, ch) in state.notes[line_start..].char_indices() {
        if remaining == 0 {
            byte = line_start + idx;
            break;
        }
        if idx + line_start >= line_end {
            byte = line_end;
            break;
        }
        remaining = remaining.saturating_sub(1);
        byte = line_start + idx + ch.len_utf8();
    }
    if remaining == 0 {
        state.notes_cursor = byte;
    } else {
        state.notes_cursor = line_end;
    }
}

fn open_archive_in_file_manager(state: &AppState) -> Result<()> {
    let path = state.archive.clone();
    if !path.exists() {
        fs::create_dir_all(&path)?;
    }

    Command::new(FILE_MANAGER_COMMAND)
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to start {FILE_MANAGER_COMMAND}"))?;

    Ok(())
}

fn open_notes_in_external_editor(state: &mut AppState) -> Result<()> {
    let goal_id = selected_goal_id(state).context("No goal selected for note editing")?;
    refresh_notes_for_selection(state)?;
    save_notes_for_selection(state)?;

    // successlib saves notes to archive/notes/goal_<id>.md
    let note_path = state
        .archive
        .join("notes")
        .join(format!("goal_{goal_id}.md"));

    let editor_value = std::env::var("EDITOR").unwrap_or_else(|_| DEFAULT_EDITOR.to_string());
    let mut editor_parts = parse_editor_command(editor_value.trim());
    if editor_parts.is_empty() {
        editor_parts.push(DEFAULT_EDITOR.to_string());
    }
    let editor_bin = editor_parts.remove(0);
    let editor_args = editor_parts;

    let note_for_run = note_path.clone();
    with_terminal_suspended(move || {
        println!("Opening notes with {} (goal {})...", editor_bin, goal_id);
        io::stdout().flush().ok();

        let mut command = Command::new(&editor_bin);
        for arg in &editor_args {
            command.arg(arg);
        }
        command.arg(&note_for_run);
        command.stdin(Stdio::inherit());
        command.stdout(Stdio::inherit());
        command.stderr(Stdio::inherit());
        let status = command
            .status()
            .with_context(|| format!("Failed to run {editor_bin}"))?;
        if !status.success() {
            bail!("Editor exited with status {status}");
        }
        Ok(())
    })?;

    // Reload notes in case the editor changed the file on disk.
    refresh_notes_for_selection(state)?;
    state.notes_cursor = state.notes.len();
    state.needs_full_redraw = true;
    Ok(())
}

fn with_terminal_suspended<F>(action: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    disable_raw_mode()?;
    {
        let mut stdout = io::stdout();
        execute!(
            stdout,
            terminal::LeaveAlternateScreen,
            event::DisableMouseCapture
        )?;
    }

    let result = action();

    {
        let mut stdout = io::stdout();
        execute!(
            stdout,
            terminal::EnterAlternateScreen,
            event::EnableMouseCapture,
            SetCursorStyle::SteadyBar,
            Clear(ClearType::All),
            MoveTo(0, 0)
        )?;
    }
    enable_raw_mode()?;

    result
}

fn parse_editor_command(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            '\\' if !in_single => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

fn parse_commands_input(input: &str) -> Vec<String> {
    input
        .split(|c| c == ';' || c == '\n')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn commands_for_goal(state: &AppState, goal_id: u64) -> Vec<String> {
    state
        .goals
        .iter()
        .find(|g| g.id == goal_id)
        .map(|g| g.commands.clone())
        .unwrap_or_default()
}

fn spawn_commands(commands: &[String]) -> Vec<SpawnedCommand> {
    commands
        .iter()
        .map(|cmd| {
            let child = {
                let mut command = Command::new("sh");
                command
                    .arg("-c")
                    .arg(format!("exec {cmd}"))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());

                // Isolate the spawned command in its own process group so we can kill everything it starts.
                unsafe {
                    command.pre_exec(|| {
                        setsid();
                        Ok(())
                    });
                }

                command.spawn()
            };

            match child {
                Ok(child) => {
                    let pgid = child.id() as i32; // after setsid(), pgid == pid
                    SpawnedCommand {
                        command: cmd.clone(),
                        child: Some(child),
                        pgid,
                    }
                }
                Err(err) => {
                    eprintln!("Failed to start '{cmd}': {err}");
                    SpawnedCommand {
                        command: cmd.clone(),
                        child: None,
                        pgid: 0,
                    }
                }
            }
        })
        .collect()
}

fn kill_spawned(spawned: &mut [SpawnedCommand]) {
    for sc in spawned.iter_mut() {
        if let Some(child) = sc.child.as_mut() {
            let pid = child.id();
            // Best-effort: kill the whole process group first so children die too.
            unsafe {
                if sc.pgid != 0 {
                    kill(-sc.pgid, SIGTERM);
                } else {
                    kill(-(pid as i32), SIGTERM);
                }
            }
            // Give the process a brief moment to exit before escalating.
            std::thread::sleep(Duration::from_millis(150));
            unsafe {
                if sc.pgid != 0 {
                    kill(-sc.pgid, libc::SIGKILL);
                } else {
                    kill(-(pid as i32), libc::SIGKILL);
                }
            }
            if let Err(err) = child.kill() {
                if err.kind() != io::ErrorKind::InvalidInput {
                    eprintln!("Failed to kill '{}': {err}", sc.command);
                }
            }
            let _ = child.wait();

            // Fallback for apps that reparent or respawn outside the pgid (e.g., GUI apps).
            let _ = Command::new("pkill")
                .arg("-f")
                .arg(&sc.command)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

fn parse_duration(input: &str) -> Option<u64> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    // Accumulate total seconds
    let mut total_seconds = 0;
    let mut current_digits = String::new();

    // Custom parser for formats like "1h30m", "90m", "1h", "90"
    for c in input.chars() {
        if c.is_ascii_digit() {
            current_digits.push(c);
        } else if c.is_alphabetic() {
            if current_digits.is_empty() {
                continue; // Ignore unit without number? or error?
            }
            let val = current_digits.parse::<u64>().ok()?;
            current_digits.clear();
            match c.to_ascii_lowercase() {
                'h' => total_seconds += val * 3600,
                'm' => total_seconds += val * 60,
                's' => total_seconds += val,
                _ => return None, // Unknown unit
            }
        } else if c.is_whitespace() {
            // Ignore whitespace
        } else {
            // Invalid character
            return None;
        }
    }

    if !current_digits.is_empty() {
        // Trailing number without unit. Assume minutes.
        let val = current_digits.parse::<u64>().ok()?;
        total_seconds += val * 60;
    }

    if total_seconds == 0 {
        None
    } else {
        Some(total_seconds)
    }
}

fn format_day_label(day: NaiveDate) -> String {
    let today = Local::now().date_naive();
    let base = day.format("%Y-%m-%d").to_string();
    let diff = (today - day).num_days();
    if diff == 0 {
        format!("{base} (today)")
    } else {
        format!("{base} (-{diff}d)")
    }
}

fn format_duration_suggestion(duration_mins: i64) -> String {
    if duration_mins == 0 {
        return "1s".to_string();
    }
    let mins = duration_mins.max(0) as i64;
    if mins == 0 {
        return "1s".to_string();
    }
    let h = mins / 60;
    let m = mins % 60;
    if h > 0 && m > 0 {
        format!("{}h {}m", h, m)
    } else if h > 0 {
        format!("{}h", h)
    } else {
        format!("{}m", m)
    }
}

fn persist_config(archive: &Path) -> Result<()> {
    let path = config_path()?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let cfg = CliConfig {
        archive: Some(archive.to_path_buf()),
    };
    let content = serde_json::to_string_pretty(&cfg)?;
    fs::write(&path, content)?;
    Ok(())
}

fn resolve_archive_interactive(preferred: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = preferred {
        return Ok(path);
    }
    let cfg_path = config_path()?;
    if cfg_path.exists() {
        if let Ok(content) = fs::read_to_string(&cfg_path) {
            if let Ok(cfg) = serde_json::from_str::<CliConfig>(&content) {
                if let Some(p) = cfg.archive {
                    return Ok(p);
                }
            }
        }
    }
    println!("Archive folder not set. Enter a path to use (will be created if missing):");
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        bail!("Archive folder not provided");
    }
    let path = PathBuf::from(trimmed);
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn config_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set; please set HOME")?;
    Ok(Path::new(&home).join(".config/success-cli/config.json"))
}
