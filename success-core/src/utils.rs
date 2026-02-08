use chrono::{Local, NaiveDate};
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::app::AppState;
use crate::style;
use crate::types::*;
use crate::ui::{build_view_items, ViewItemKind};

pub fn is_dialog_open(mode: &Mode) -> bool {
    matches!(
        mode,
        Mode::AddSession
            | Mode::AddReward
            | Mode::GoalForm
            | Mode::QuantityDoneInput { .. }
            | Mode::DurationInput { .. }
    )
}

pub fn get_block_style(
    current: FocusedBlock,
    target: FocusedBlock,
    mode: &Mode,
) -> ratatui::style::Style {
    use ratatui::style::Style;
    if !is_dialog_open(mode) && current == target {
        Style::default().fg(style::BLUE)
    } else {
        Style::default()
    }
}

pub fn get_dimmed_style(mode: &Mode) -> ratatui::style::Style {
    use ratatui::style::Style;
    if is_dialog_open(mode) {
        Style::default().fg(style::GRAY_DIM)
    } else {
        Style::default()
    }
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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

pub fn centered_rect_fixed_height(percent_x: u16, height: u16, r: Rect) -> Rect {
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

pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
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

pub fn parse_duration(input: &str) -> Option<u64> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    let mut total_seconds = 0;
    let mut current_digits = String::new();

    for c in input.chars() {
        if c.is_ascii_digit() {
            current_digits.push(c);
        } else if c.is_alphabetic() {
            if current_digits.is_empty() {
                continue;
            }
            let val = current_digits.parse::<u64>().ok()?;
            current_digits.clear();
            match c.to_ascii_lowercase() {
                'h' => total_seconds += val * 3600,
                'm' => total_seconds += val * 60,
                's' => total_seconds += val,
                _ => return None,
            }
        } else if c.is_whitespace() {
            // ignore
        } else {
            return None;
        }
    }

    if !current_digits.is_empty() {
        let val = current_digits.parse::<u64>().ok()?;
        total_seconds += val * 60; // Assume minutes
    }

    if total_seconds == 0 {
        None
    } else {
        Some(total_seconds)
    }
}

pub fn format_day_label(day: NaiveDate) -> String {
    let today = Local::now().date_naive();
    let base = day.format("%Y-%m-%d").to_string();
    let diff = (today - day).num_days();
    if diff == 0 {
        format!("{base}, today")
    } else {
        format!("{base}, -{diff}d")
    }
}

pub fn format_duration_suggestion(duration_mins: i64) -> String {
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

pub fn parse_commands_input(input: &str) -> Vec<String> {
    input
        .split([';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

pub fn parse_optional_u32(input: &str) -> Option<u32> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        None
    } else {
        trimmed.parse::<u32>().ok()
    }
}

pub fn selected_goal_id(state: &AppState) -> Option<u64> {
    let items = build_view_items(state, 20);
    match items.get(state.selected).map(|v| v.kind) {
        Some(ViewItemKind::RunningTimer) => state.timer.as_ref().map(|t| t.goal_id),
        Some(ViewItemKind::Existing(_, idx)) => state.nodes.get(idx).map(|n| n.goal_id),
        _ => state.timer.as_ref().map(|t| t.goal_id),
    }
}

pub fn goal_quantity_name(state: &AppState, goal_id: u64) -> Option<String> {
    state
        .goals
        .iter()
        .find(|g| g.id == goal_id)
        .and_then(|g| g.quantity_name.clone())
}
