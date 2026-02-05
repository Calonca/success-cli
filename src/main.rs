#[cfg(unix)]
use libc::{kill, setsid, SIGTERM};
use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::Duration as ChronoDuration;
use chrono::{DateTime, Local, NaiveDate, Utc};
use clap::Parser;
use crossterm::cursor::{MoveTo, SetCursorStyle};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};
use crossterm::{execute, terminal};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
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

fn render_goal_form_dialog(f: &mut ratatui::Frame, state: &AppState) {
    if let Some(form) = &state.form_state {
        let area = centered_rect(80, 70, f.size());
        f.render_widget(ratatui::widgets::Clear, area);

        let title = if form.is_reward {
            "Create new reward"
        } else {
            "Create new goal"
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Blue));

        let inner = block.inner(area);
        f.render_widget(block, area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Goal Name
                Constraint::Length(1), // Quantity
                Constraint::Length(1), // Commands
                Constraint::Length(1), // Spacer
                Constraint::Min(1),    // Filler
                Constraint::Length(1), // Help text
            ])
            .split(inner);

        let name_prefix = "Name: ";
        let name_style = if form.current_field == FormField::GoalName {
            Style::default().fg(Color::Blue)
        } else {
            Style::default()
        };
        let name_line = format!("{}{}", name_prefix, form.goal_name.value);
        f.render_widget(Paragraph::new(name_line).style(name_style), layout[0]);

        let qty_prefix = "Quantity name (optional): ";
        let qty_style = if form.current_field == FormField::Quantity {
            Style::default().fg(Color::Blue)
        } else {
            Style::default()
        };
        let qty_line = format!("{}{}", qty_prefix, form.quantity_name.value);
        f.render_widget(Paragraph::new(qty_line).style(qty_style), layout[1]);

        let cmd_prefix = "Commands (optional, separated by ;): ";
        let cmd_style = if form.current_field == FormField::Commands {
            Style::default().fg(Color::Blue)
        } else {
            Style::default()
        };
        let cmd_line = format!("{}{}", cmd_prefix, form.commands.value);
        f.render_widget(Paragraph::new(cmd_line).style(cmd_style), layout[2]);

        let help = Paragraph::new("↑↓/Tab: navigate • Enter: create • Esc: cancel")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(help, layout[5]);

        match form.current_field {
            FormField::GoalName => {
                let cursor_x =
                    layout[0].x + name_prefix.len() as u16 + form.goal_name.cursor as u16;
                let cursor_y = layout[0].y;
                f.set_cursor(cursor_x, cursor_y);
            }
            FormField::Quantity => {
                let cursor_x =
                    layout[1].x + qty_prefix.len() as u16 + form.quantity_name.cursor as u16;
                let cursor_y = layout[1].y;
                f.set_cursor(cursor_x, cursor_y);
            }
            FormField::Commands => {
                let cursor_x = layout[2].x + cmd_prefix.len() as u16 + form.commands.cursor as u16;
                let cursor_y = layout[2].y;
                f.set_cursor(cursor_x, cursor_y);
            }
        }
    }
}

fn render_duration_input_dialog(f: &mut ratatui::Frame, state: &AppState) {
    if let Mode::DurationInput { ref goal_name, .. } = state.mode {
        // Height 4: Border(1) + Input(1) + Help(1) + Border(1)
        let area = centered_rect_fixed_height(60, 4, f.size());
        f.render_widget(ratatui::widgets::Clear, area);

        let title = format!("Duration for {} (e.g., 30m, 1h)", goal_name);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Blue));

        let inner = block.inner(area);
        f.render_widget(block, area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Input
                Constraint::Min(1),    // Help
            ])
            .split(inner);

        let input_line = format!("> {}", state.duration_input.value);
        f.render_widget(Paragraph::new(input_line.clone()), layout[0]);

        // Help hint in last line of block
        f.render_widget(
            Paragraph::new("Enter: start • Esc: cancel")
                .style(Style::default().fg(Color::DarkGray)),
            layout[1],
        );

        let cursor_x = inner.x + 2 + state.duration_input.cursor as u16;
        let cursor_y = inner.y;
        f.set_cursor(cursor_x, cursor_y);
    }
}

fn render_quantity_input_dialog(f: &mut ratatui::Frame, state: &AppState) {
    if let Mode::QuantityDoneInput {
        ref goal_name,
        ref quantity_name,
    } = state.mode
    {
        // Height 4 for consistent look
        let area = centered_rect_fixed_height(60, 4, f.size());
        f.render_widget(ratatui::widgets::Clear, area);

        let title = if let Some(name) = quantity_name {
            format!("{} done for {} (blank to skip)", name, goal_name)
        } else {
            format!("Quantity done for {} (blank to skip)", goal_name)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Blue));

        let inner = block.inner(area);
        f.render_widget(block, area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Input
                Constraint::Min(1),    // Help
            ])
            .split(inner);

        let input_line = format!("> {}", state.quantity_input.value);
        f.render_widget(Paragraph::new(input_line.clone()), layout[0]);

        f.render_widget(
            Paragraph::new("Enter: confirm • Esc: skip")
                .style(Style::default().fg(Color::DarkGray)),
            layout[1],
        );

        let cursor_x = inner.x + 2 + state.quantity_input.cursor as u16;
        let cursor_y = inner.y;
        f.set_cursor(cursor_x, cursor_y);
    }
}

/// Returns a centered rect of the given percentage size within the parent rect
fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    r: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    use ratatui::layout::{Constraint, Direction, Layout};

    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn centered_rect_fixed_height(
    percent_x: u16,
    height: u16,
    r: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    use ratatui::layout::{Constraint, Direction, Layout};

    let vertical_pad = r.height.saturating_sub(height) / 2;

    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(vertical_pad),
            Constraint::Length(height),
            Constraint::Length(vertical_pad),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Returns a centered rect of fixed size (width, height) within the parent rect
#[allow(dead_code)]
fn centered_rect_fixed(width: u16, height: u16, r: ratatui::layout::Rect) -> ratatui::layout::Rect {
    use ratatui::layout::{Constraint, Direction, Layout};

    let vertical_padding = r.height.saturating_sub(height) / 2;
    let horizontal_padding = r.width.saturating_sub(width) / 2;

    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(vertical_padding),
            Constraint::Length(height),
            Constraint::Length(vertical_padding),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(horizontal_padding),
            Constraint::Length(width),
            Constraint::Length(horizontal_padding),
        ])
        .split(popup_layout[1])[1]
}

/// Word-wrap a single line of text to fit within `width` characters.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            if word.chars().count() > width {
                // Break a single long word across lines
                let mut chars = word.chars();
                while chars.as_str().chars().count() > 0 {
                    let chunk: String = chars.by_ref().take(width).collect();
                    if chunk.is_empty() {
                        break;
                    }
                    lines.push(chunk);
                }
            } else {
                current = word.to_string();
            }
        } else if current.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut current));
            if word.chars().count() > width {
                let mut chars = word.chars();
                while chars.as_str().chars().count() > 0 {
                    let chunk: String = chars.by_ref().take(width).collect();
                    if chunk.is_empty() {
                        break;
                    }
                    lines.push(chunk);
                }
            } else {
                current = word.to_string();
            }
        } else {
            current.push(' ');
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    fn new(value: String) -> Self {
        let len = value.chars().count();
        Self { value, cursor: len }
    }

    fn from_string(value: String) -> Self {
        Self::new(value)
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.insert_char(c);
                true
            }
            KeyCode::Backspace => {
                self.delete_char_back();
                true
            }
            KeyCode::Delete => {
                self.delete_char_forward();
                true
            }
            KeyCode::Left => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.move_word_left();
                } else {
                    self.move_left();
                }
                true
            }
            KeyCode::Right => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.move_word_right();
                } else {
                    self.move_right();
                }
                true
            }
            KeyCode::Home => {
                self.cursor = 0;
                true
            }
            KeyCode::End => {
                self.cursor = self.value.chars().count();
                true
            }
            _ => false,
        }
    }

    fn insert_char(&mut self, c: char) {
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

    fn delete_char_back(&mut self) {
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

    fn delete_char_forward(&mut self) {
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

    fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn move_right(&mut self) {
        if self.cursor < self.value.chars().count() {
            self.cursor += 1;
        }
    }

    fn move_word_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.value.chars().collect();
        let mut idx = self.cursor;
        // Skip current non-separators if we are at the end of a word?
        // Simple logic: skip spaces backwards, then skip non-spaces backwards
        while idx > 0 && idx <= chars.len() && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        while idx > 0 && idx <= chars.len() && !chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        self.cursor = idx;
    }

    fn move_word_right(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let len = chars.len();
        if self.cursor >= len {
            return;
        }
        let mut idx = self.cursor;
        // Skip current non-separators
        while idx < len && !chars[idx].is_whitespace() {
            idx += 1;
        }
        // Skip spaces
        while idx < len && chars[idx].is_whitespace() {
            idx += 1;
        }
        self.cursor = idx;
    }

    fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
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
enum FormField {
    #[default]
    GoalName,
    Quantity,
    Commands,
}

#[derive(Debug, Clone, Default)]
struct FormState {
    current_field: FormField,
    goal_name: TextInput,
    quantity_name: TextInput,
    commands: TextInput,
    is_reward: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum FocusedBlock {
    #[default]
    SessionsList,
    Notes,
}

fn is_dialog_open(mode: &Mode) -> bool {
    matches!(
        mode,
        Mode::AddSession
            | Mode::AddReward
            | Mode::GoalForm
            | Mode::QuantityDoneInput { .. }
            | Mode::DurationInput { .. }
    )
}

fn get_block_style(current: FocusedBlock, target: FocusedBlock, mode: &Mode) -> Style {
    if !is_dialog_open(mode) && current == target {
        Style::default().fg(Color::Blue)
    } else {
        Style::default()
    }
}

fn get_dimmed_style(mode: &Mode) -> Style {
    if is_dialog_open(mode) {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    }
}

fn get_cursor_style(mode: &Mode) -> SetCursorStyle {
    match mode {
        Mode::NotesEdit
        | Mode::AddSession
        | Mode::AddReward
        | Mode::GoalForm
        | Mode::QuantityDoneInput { .. }
        | Mode::DurationInput { .. } => SetCursorStyle::SteadyBlock,
        Mode::View | Mode::Timer => SetCursorStyle::SteadyBlock,
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
    search_input: TextInput,
    search_selected: usize,
    duration_input: TextInput,
    quantity_input: TextInput,
    timer: Option<TimerState>,
    pending_session: Option<PendingSession>,
    notes: String,
    notes_cursor: usize,
    needs_full_redraw: bool,
    focused_block: FocusedBlock,
    form_state: Option<FormState>,
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

#[derive(Debug, Clone)]
struct PendingSession {
    label: String,
    goal_id: u64,
    total: u64,
    is_reward: bool,
    started_at: DateTime<Utc>,
}

#[derive(Debug)]
struct SpawnedCommand {
    command: String,
    child: Option<Child>,
    pgid: i32,
}

#[derive(Parser, Debug)]
#[command(name = "success-cli")]
#[command(about = "CLI for achieving goals", long_about = None)]
struct Args {
    /// Custom archive path (useful for testing)
    #[arg(short, long)]
    archive: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let archive = resolve_archive_interactive(args.archive.clone())?;
    let today = Local::now().date_naive();
    let mut state = AppState {
        archive: archive.clone(),
        goals: list_goals(&archive, None)?,
        nodes: list_day_sessions(&archive, today)?,
        current_day: today,
        selected: 0,
        mode: Mode::View,
        search_input: TextInput::default(),
        search_selected: 0,
        duration_input: TextInput::default(),
        quantity_input: TextInput::default(),
        timer: None,

        pending_session: None,
        notes: String::new(),
        notes_cursor: 0,
        needs_full_redraw: false,
        focused_block: FocusedBlock::SessionsList,
        form_state: None,
    };

    // Start focused on the most recent item (last in list) if any exist.
    state.selected = build_view_items(&state, 20).len().saturating_sub(1);

    refresh_notes_for_selection(&mut state)?;

    // Only persist config if archive wasn't provided via CLI argument
    if args.archive.is_none() {
        persist_config(&archive).ok();
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        terminal::EnterAlternateScreen,
        event::EnableMouseCapture,
        SetCursorStyle::SteadyBlock
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, &mut state);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        terminal::LeaveAlternateScreen,
        event::DisableMouseCapture,
        SetCursorStyle::DefaultUserShape
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Error: {err}");
    }
    Ok(())
}

fn run_app<B: ratatui::backend::Backend + std::io::Write>(
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
        execute!(terminal.backend_mut(), get_cursor_style(&state.mode))?;

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

fn render_goal_selector_dialog(f: &mut ratatui::Frame, state: &AppState) {
    if !matches!(state.mode, Mode::AddSession | Mode::AddReward) {
        return;
    }

    let popup_area = centered_rect(80, 70, f.size());
    f.render_widget(ratatui::widgets::Clear, popup_area);

    let prompt = if matches!(state.mode, Mode::AddReward) {
        "Choose reward"
    } else {
        "Choose goal"
    };

    let popup_block = Block::default()
        .borders(Borders::ALL)
        .title(prompt)
        .border_style(Style::default().fg(Color::Blue));

    let inner = popup_block.inner(popup_area);
    f.render_widget(popup_block, popup_area);

    let dialog_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Input
            Constraint::Min(1),    // List
            Constraint::Length(1), // Help
        ])
        .split(inner);

    let input_line = format!("> {}", state.search_input.value);
    let input_para = Paragraph::new(input_line.clone());
    f.render_widget(input_para, dialog_chunks[0]);

    let results = search_results(state);
    let list_items: Vec<ListItem> = results
        .iter()
        .map(|(label, _)| ListItem::new(Line::from(label.clone())))
        .collect();

    let mut list_state = ListState::default();
    if !results.is_empty() {
        list_state.select(Some(state.search_selected.min(results.len() - 1)));
    }

    let list = List::new(list_items).highlight_style(
        Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
    );

    f.render_stateful_widget(list, dialog_chunks[1], &mut list_state);

    f.render_widget(
        Paragraph::new("Type to search • ↑↓ select • Enter pick • Esc cancel")
            .style(Style::default().fg(Color::DarkGray)),
        dialog_chunks[2],
    );

    let cursor_x = dialog_chunks[0].x + 2 + state.search_input.cursor as u16;
    let cursor_y = dialog_chunks[0].y;
    f.set_cursor(
        cursor_x.min(dialog_chunks[0].x + dialog_chunks[0].width.saturating_sub(1)),
        cursor_y,
    );
}

fn ui(f: &mut ratatui::Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(5),    // Body
        ])
        .split(f.size());

    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[1]);

    let header_line = format!("Archive: {} (open with 'o')", state.archive.display());

    let dimmed = get_dimmed_style(&state.mode);

    let header = Paragraph::new(header_line)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Success CLI")
                .style(dimmed),
        )
        .style(dimmed);

    f.render_widget(header, chunks[0]);

    let list_width = body_chunks[0].width.saturating_sub(4) as usize;

    let items = build_view_items(state, list_width);

    let is_any_timer_selected = items
        .get(state.selected)
        .map(|it| it.kind == ViewItemKind::RunningTimer)
        .unwrap_or(false);

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == state.selected;
            let is_dialog_open = is_dialog_open(&state.mode);

            // Highlight selected item even if dialog is open (as requested),
            // but the list itself might be dimmed. Explicit style overrides list style.
            // Also highlight ALL timer items if ANY timer item is selected.
            // Also highlight the [Insert] line if we are in QuantityDoneInput mode.
            let is_quantity_input = matches!(state.mode, Mode::QuantityDoneInput { .. });
            let is_insert_item = matches!(
                item.kind,
                ViewItemKind::AddSession | ViewItemKind::AddReward
            );

            let should_highlight = is_selected
                || (is_any_timer_selected && item.kind == ViewItemKind::RunningTimer)
                || (is_quantity_input && is_insert_item);

            let label_style = if should_highlight {
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            // Split label by newlines, then word-wrap each line to fit the list width
            let label_lines: Vec<&str> = item.label.lines().collect();
            let mut lines: Vec<Line> = label_lines
                .iter()
                .flat_map(|line_text| {
                    wrap_text(line_text, list_width)
                        .into_iter()
                        .enumerate()
                        .map(|(wrap_idx, wrapped)| {
                            let text = if wrap_idx == 0 {
                                wrapped // First wrapped line, no indentation
                            } else {
                                format!("    {}", wrapped) // Continuation lines get 4 spaces
                            };
                            Line::from(vec![Span::styled(text, label_style)])
                        })
                })
                .collect();

            // Add hints based on item type
            if is_selected && !is_dialog_open {
                // For multiline items (RunningTimer), add hints to the first line
                // For single-line items, add hints to the last line
                let target_line_idx = if item.kind == ViewItemKind::RunningTimer && lines.len() > 1
                {
                    0 // First line for timer
                } else {
                    if lines.is_empty() {
                        0
                    } else {
                        lines.len() - 1
                    }
                };

                let hint_spans = match item.kind {
                    ViewItemKind::AddSession => {
                        vec![Span::styled(
                            " (Enter: add session)",
                            Style::default().fg(Color::DarkGray),
                        )]
                    }
                    ViewItemKind::AddReward => {
                        vec![Span::styled(
                            " (Enter: receive reward)",
                            Style::default().fg(Color::DarkGray),
                        )]
                    }
                    ViewItemKind::Existing(_, _) | ViewItemKind::RunningTimer => {
                        if state.focused_block == FocusedBlock::SessionsList {
                            vec![Span::styled(
                                " (e: edit, E: external edit)",
                                Style::default().fg(Color::DarkGray),
                            )]
                        } else {
                            vec![]
                        }
                    }
                };

                if !hint_spans.is_empty() {
                    if let Some(target_line) = lines.get_mut(target_line_idx) {
                        target_line.spans.extend(hint_spans);
                    }
                }
            }

            ListItem::new(lines)
        })
        .collect();
    let title = format!(
        "Sessions of {} (←→ day • ↑↓ move)",
        format_day_label(state.current_day)
    );

    let sessions_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(dimmed)
        .border_style(get_block_style(
            state.focused_block,
            FocusedBlock::SessionsList,
            &state.mode,
        ));

    let list = List::new(list_items).block(sessions_block).style(dimmed);

    let mut stateful = ratatui::widgets::ListState::default();
    if !items.is_empty() {
        stateful.select(Some(state.selected.min(items.len() - 1)));
    }
    f.render_stateful_widget(list, body_chunks[0], &mut stateful);

    let notes_title = if matches!(state.mode, Mode::NotesEdit) {
        "Notes (Esc to stop editing)"
    } else {
        "Notes"
    };

    let notes_block = Block::default()
        .borders(Borders::ALL)
        .title(notes_title)
        .style(dimmed)
        .border_style(get_block_style(
            state.focused_block,
            FocusedBlock::Notes,
            &state.mode,
        ));

    if selected_goal_id(state).is_some() {
        let (cursor_line, cursor_col) = notes_cursor_line_col(state);
        let view_height = body_chunks[1].height.max(1) as usize;
        let desired_mid = view_height / 2;
        let offset = cursor_line.saturating_sub(desired_mid);
        let offset_u16 = offset.min(u16::MAX as usize) as u16;
        let notes_para = Paragraph::new(state.notes.clone())
            .block(notes_block)
            .style(dimmed)
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
        let notes_para = Paragraph::new("Select a task to view notes")
            .block(notes_block)
            .style(dimmed);
        f.render_widget(notes_para, body_chunks[1]);
    }

    render_goal_selector_dialog(f, state);
    render_goal_form_dialog(f, state);
    render_duration_input_dialog(f, state);
    render_quantity_input_dialog(f, state);
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

fn build_timer_view_items(timer: &TimerState, width: usize) -> Vec<ViewItem> {
    let started_local = timer.started_at.with_timezone(&Local).format("%H:%M");
    let info_line = format!(
        "[*] {} ({}s left) [started {}]",
        timer.label, timer.remaining, started_local
    );

    let pct = if timer.total == 0 {
        0.0
    } else {
        1.0 - (timer.remaining as f32 / timer.total as f32)
    };
    let ratio = pct.clamp(0.0, 1.0);

    // Ensure width is at least something reasonable to avoid panic or weirdness
    let bar_width = width.max(1);
    let filled = (ratio * bar_width as f32) as usize;
    let empty = bar_width.saturating_sub(filled);
    let bar_line = format!("[{}{}]", "█".repeat(filled), "░".repeat(empty));

    let combined_label = format!("{}\n{}", info_line, bar_line);

    vec![ViewItem {
        label: combined_label,
        kind: ViewItemKind::RunningTimer,
    }]
}

fn build_view_items(state: &AppState, width: usize) -> Vec<ViewItem> {
    let mut items = Vec::new();
    for (idx, n) in state.nodes.iter().enumerate() {
        let prefix = match n.kind {
            SessionKind::Goal => "[S]",
            SessionKind::Reward => "[R]",
        };
        let duration = (n.end_at - n.start_at).num_minutes();
        let times = get_formatted_session_time_range(n);
        let unit = goal_quantity_name(state, n.goal_id)
            .map(|u| format!(" {u}"))
            .unwrap_or_default();
        let qty_label = n
            .quantity
            .map(|q| format!("{q}{unit} in "))
            .unwrap_or_default();
        items.push(ViewItem {
            label: format!("{prefix} {} ({qty_label}{duration}m) [{times}]", n.name),
            kind: ViewItemKind::Existing(n.kind, idx),
        });
    }
    // Only show the running timer if we're viewing today.
    if let Some(timer) = &state.timer {
        if state.current_day == Local::now().date_naive() {
            items.extend(build_timer_view_items(timer, width));
        }
    }

    // Only offer add rows when no timer is running AND we are viewing today.
    if state.timer.is_none() && state.current_day == Local::now().date_naive() {
        if let Mode::QuantityDoneInput {
            ref goal_name,
            ref quantity_name,
        } = state.mode
        {
            let quantity_name = quantity_name.as_deref().unwrap_or("quantity");
            items.push(ViewItem {
                label: format!("[+] Insert {quantity_name} for {goal_name}"),
                kind: ViewItemKind::AddSession,
            });
        } else if state
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
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        return Ok(true);
    }
    match state.mode {
        Mode::View => handle_view_key(state, key),
        Mode::AddSession | Mode::AddReward => handle_search_key(state, key),
        Mode::GoalForm => handle_form_key(state, key),
        Mode::QuantityDoneInput { .. } => handle_quantity_done_key(state, key),
        Mode::DurationInput { .. } => handle_duration_key(state, key),
        Mode::Timer => handle_timer_key(state, key),
        Mode::NotesEdit => handle_notes_key(state, key),
    }
}

fn handle_form_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    let Some(form) = state.form_state.as_mut() else {
        state.mode = Mode::View;
        return Ok(false);
    };

    let field = match form.current_field {
        FormField::GoalName => &mut form.goal_name,
        FormField::Quantity => &mut form.quantity_name,
        FormField::Commands => &mut form.commands,
    };

    if field.handle_key(key) {
        return Ok(false);
    }

    match key.code {
        KeyCode::Esc => {
            state.form_state = None;
            state.mode = Mode::View;
        }
        KeyCode::Up | KeyCode::BackTab => {
            form.current_field = match form.current_field {
                FormField::GoalName => FormField::Commands,
                FormField::Quantity => FormField::GoalName,
                FormField::Commands => FormField::Quantity,
            };
        }
        KeyCode::Down | KeyCode::Tab => {
            form.current_field = match form.current_field {
                FormField::GoalName => FormField::Quantity,
                FormField::Quantity => FormField::Commands,
                FormField::Commands => FormField::GoalName,
            };
        }
        KeyCode::Enter => {
            let name = form.goal_name.value.trim();
            if name.is_empty() {
                return Ok(false);
            }

            let commands = parse_commands_input(&form.commands.value);
            let quantity_name = if form.quantity_name.value.trim().is_empty() {
                None
            } else {
                Some(form.quantity_name.value.trim().to_string())
            };
            let is_reward = form.is_reward;
            let goal_name = name.to_string();

            let created = add_goal(
                &state.archive,
                &goal_name,
                is_reward,
                commands,
                quantity_name,
            )?;
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
    Ok(false)
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
            let max_idx = build_view_items(state, 20).len().saturating_sub(1);
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
                state.focused_block = FocusedBlock::Notes;
            }
        }
        KeyCode::Char('E') => {
            if selected_goal_id(state).is_some() {
                let _ = open_notes_in_external_editor(state);
            }
        }
        KeyCode::Char('o') => {
            let _ = open_archive_in_file_manager(state);
        }
        KeyCode::Enter => {
            let items = build_view_items(state, 20);
            let Some(item) = items.get(state.selected) else {
                return Ok(false);
            };
            match item.kind {
                ViewItemKind::AddSession => {
                    if state.timer.is_some() {
                        return Ok(false);
                    }
                    state.mode = Mode::AddSession;
                    state.search_input.clear();
                    state.search_selected = 0;
                }
                ViewItemKind::AddReward => {
                    if state.timer.is_some() {
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
        return Ok(());
    };
    if new_day > today {
        return Ok(());
    }

    state.current_day = new_day;
    state.nodes = list_day_sessions(&state.archive, new_day)?;
    state.selected = build_view_items(state, 20).len().saturating_sub(1);
    refresh_notes_for_selection(state)?;
    Ok(())
}

fn handle_search_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    if state.search_input.handle_key(key) {
        state.search_selected = 0;
        return Ok(false);
    }
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
        _ => {}
    }
    Ok(false)
}

fn handle_quantity_done_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    let Mode::QuantityDoneInput { .. } = state.mode else {
        return Ok(false);
    };

    if state.quantity_input.handle_key(key) {
        return Ok(false);
    }

    match key.code {
        KeyCode::Esc => {
            state.quantity_input.clear();
            if let Some(pending) = state.pending_session.take() {
                finalize_session(state, pending, None);
            } else {
                state.mode = Mode::View;
            }
        }
        KeyCode::Enter => {
            let qty = parse_optional_u32(&state.quantity_input.value);

            if let Some(pending) = state.pending_session.take() {
                finalize_session(state, pending, qty);
            }
            state.quantity_input.clear();
        }
        _ => {}
    }

    Ok(false)
}

fn handle_duration_key(state: &mut AppState, key: KeyEvent) -> Result<bool> {
    if state.duration_input.handle_key(key) {
        return Ok(false);
    }
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
            let secs = parse_duration(&state.duration_input.value).unwrap_or(25 * 60);
            start_timer(state, goal_name.clone(), goal_id, secs as u32, is_reward)?;
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
            state.focused_block = FocusedBlock::SessionsList;
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
        // If user is editing notes, save them first
        if matches!(state.mode, Mode::NotesEdit) {
            if let Err(e) = save_notes_for_selection(state) {
                eprintln!("Failed to save notes: {e}");
            }
        }

        // Only kill spawned apps when finishing a reward, not a regular session
        if timer.is_reward {
            kill_spawned(&mut timer.spawned);
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

fn start_timer(
    state: &mut AppState,
    goal_name: String,
    goal_id: u64,
    seconds: u32,
    is_reward: bool,
) -> Result<()> {
    if state.timer.is_some() {
        return Ok(());
    }

    let today = Local::now().date_naive();
    if state.current_day != today {
        state.current_day = today;
        state.nodes = list_day_sessions(&state.archive, today)?;
        state.selected = build_view_items(state, 20).len().saturating_sub(1);
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
    state.selected = build_view_items(state, 20).len().saturating_sub(1);
    if let Err(e) = append_session_start_header(&state.archive, goal_id, started_at) {
        eprintln!("Failed to prepare notes: {e}");
    }
    refresh_notes_for_selection(state)?;
    state.mode = Mode::Timer;
    Ok(())
}

fn finalize_session(state: &mut AppState, pending: PendingSession, quantity: Option<u32>) {
    // If user is editing notes, save and exit notes mode first
    if matches!(state.mode, Mode::NotesEdit) {
        if let Err(e) = save_notes_for_selection(state) {
            eprintln!("Failed to save notes: {e}");
        }
    }

    state.mode = Mode::View;
    state.focused_block = FocusedBlock::SessionsList;
    let duration_secs = pending.total.min(u32::MAX as u64) as u32;
    let created = match add_session(
        &state.archive,
        pending.goal_id,
        &pending.label,
        pending.started_at,
        duration_secs,
        pending.is_reward,
        quantity,
    ) {
        Ok(session) => session,
        Err(e) => {
            eprintln!("Failed to record session: {e}");
            return;
        }
    };

    let timer_day = created.start_at.with_timezone(&Local).date_naive();
    if state.current_day == timer_day {
        match list_day_sessions(&state.archive, timer_day) {
            Ok(nodes) => {
                state.nodes = nodes;
                let items = build_view_items(state, 20);
                state.selected = items.len().saturating_sub(1);
                if let Err(e) = refresh_notes_for_selection(state) {
                    eprintln!("Failed to load notes: {e}");
                }
            }
            Err(e) => eprintln!("Failed to load day graph: {e}"),
        }
    }
}

fn parse_optional_u32(input: &str) -> Option<u32> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        None
    } else {
        trimmed.parse::<u32>().ok()
    }
}

fn goal_quantity_name(state: &AppState, goal_id: u64) -> Option<String> {
    state
        .goals
        .iter()
        .find(|g| g.id == goal_id)
        .and_then(|g| g.quantity_name.clone())
}

fn search_results(state: &AppState) -> Vec<(String, SearchResult)> {
    let q = state.search_input.value.trim();

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
    let items = build_view_items(state, 20);
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
        .find(|(i, _)| *i >= state.notes_cursor)
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
        if let Some(ch) = state.notes[..idx].chars().next_back() {
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
        if let Some(ch) = state.notes[..idx].chars().next_back() {
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
            SetCursorStyle::SteadyBlock,
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
        .split([';', '\n'])
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

#[cfg(unix)]
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

#[cfg(not(unix))]
fn spawn_commands(commands: &[String]) -> Vec<SpawnedCommand> {
    commands
        .iter()
        .map(|cmd| {
            let child = if cfg!(target_os = "windows") {
                let mut command = Command::new("cmd");
                command
                    .arg("/C")
                    .arg(cmd)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            } else {
                let mut command = Command::new("sh");
                command
                    .arg("-c")
                    .arg(format!("exec {cmd}"))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            };

            match child {
                Ok(child) => SpawnedCommand {
                    command: cmd.clone(),
                    child: Some(child),
                    pgid: 0,
                },
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

#[cfg(unix)]
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

#[cfg(not(unix))]
fn kill_spawned(spawned: &mut [SpawnedCommand]) {
    for sc in spawned.iter_mut() {
        if let Some(mut child) = sc.child.take() {
            if let Err(err) = child.kill() {
                if err.kind() != io::ErrorKind::InvalidInput {
                    eprintln!("Failed to kill '{}': {err}", sc.command);
                }
            }
            let _ = child.wait();
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
        format!("{base}, today")
    } else {
        format!("{base}, -{diff}d")
    }
}

fn format_duration_suggestion(duration_mins: i64) -> String {
    if duration_mins == 0 {
        return "1s".to_string();
    }
    let mins = duration_mins.max(0);
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
