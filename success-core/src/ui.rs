use chrono::Local;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph};

use crate::app::AppState;
use crate::handlers::search_results;
use crate::style;
use crate::types::*;
use crate::utils::*;
use successlib::{SessionKind, SessionView};
use tui_textarea::TextArea;

// ── View items ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ViewItem {
    pub label: String,
    pub kind: ViewItemKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewItemKind {
    RunningTimer,
    Existing(SessionKind, usize),
    AddSession,
    AddReward,
}

fn build_timer_view_items(timer: &TimerState, _width: usize) -> Vec<ViewItem> {
    let started_local = timer.started_at.with_timezone(&Local).format("%H:%M");
    let info_line = format!(
        "[*] {} ({}s left) [started {}]",
        timer.label, timer.remaining, started_local
    );

    vec![ViewItem {
        label: info_line,
        kind: ViewItemKind::RunningTimer,
    }]
}

fn get_formatted_session_time_range(n: &SessionView) -> String {
    let start = chrono::DateTime::from_timestamp(n.start_at, 0)
        .map(|dt| dt.with_timezone(&Local).format("%H:%M").to_string())
        .unwrap_or_else(|| "??:??".to_string());
    let end = chrono::DateTime::from_timestamp(n.end_at, 0)
        .map(|dt| dt.with_timezone(&Local).format("%H:%M").to_string())
        .unwrap_or_else(|| "??:??".to_string());
    format!("{start}-{end}")
}

pub fn build_view_items(state: &AppState, width: usize) -> Vec<ViewItem> {
    let mut items = Vec::new();
    for (idx, n) in state.nodes.iter().enumerate() {
        let prefix = match n.kind {
            SessionKind::Goal => "[S]",
            SessionKind::Reward => "[R]",
        };
        let duration = (n.end_at - n.start_at) / 60;
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

    if let Some(timer) = &state.timer {
        if state.current_day == Local::now().date_naive() {
            items.extend(build_timer_view_items(timer, width));
        }
    }

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

// ── Main UI ──────────────────────────────────────────────────────────────

/// Render the entire UI.
///
/// `header_text` is the text shown in the header bar (e.g. "Archive: /path (open with 'o')").
pub fn ui(f: &mut ratatui::Frame, state: &AppState, header_text: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(5),    // Body
        ])
        .split(f.area());

    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[1]);

    let dimmed = get_dimmed_style(&state.mode);

    let header = Paragraph::new(Line::from(header_text.to_string()))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Success CLI")
                .style(dimmed),
        )
        .style(dimmed);

    f.render_widget(header, chunks[0]);

    let (list_area, gauge_area) = if state.timer.is_some()
        && state.current_day == Local::now().date_naive()
        && body_chunks[0].height > 4
    {
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(body_chunks[0]);
        (areas[0], Some(areas[1]))
    } else {
        (body_chunks[0], None)
    };

    let list_width = list_area.width.saturating_sub(4) as usize;

    let items = build_view_items(state, list_width);

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == state.selected;
            let is_dlg_open = is_dialog_open(&state.mode);

            let is_quantity_input = matches!(state.mode, Mode::QuantityDoneInput { .. });
            let is_insert_item = matches!(
                item.kind,
                ViewItemKind::AddSession | ViewItemKind::AddReward
            );

            let should_highlight = is_selected || (is_quantity_input && is_insert_item);

            let label_style = if should_highlight {
                Style::default()
                    .fg(style::BLUE)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let label_lines: Vec<&str> = item.label.lines().collect();
            let mut lines: Vec<Line> = label_lines
                .iter()
                .flat_map(|line_text| {
                    wrap_text(line_text, list_width)
                        .into_iter()
                        .enumerate()
                        .map(|(wrap_idx, wrapped)| {
                            let text = if wrap_idx == 0 {
                                wrapped
                            } else {
                                format!("    {}", wrapped)
                            };
                            Line::from(vec![Span::styled(text, label_style)])
                        })
                        .collect::<Vec<Line>>()
                })
                .collect();

            if is_selected && !is_dlg_open {
                let target_line_idx = if lines.is_empty() { 0 } else { lines.len() - 1 };

                let hint_spans = match item.kind {
                    ViewItemKind::AddSession => {
                        vec![Span::styled(
                            " (Enter: add session)",
                            Style::default().fg(style::GRAY_DIM),
                        )]
                    }
                    ViewItemKind::AddReward => {
                        vec![Span::styled(
                            " (Enter: receive reward)",
                            Style::default().fg(style::GRAY_DIM),
                        )]
                    }
                    ViewItemKind::Existing(_, _) | ViewItemKind::RunningTimer => {
                        if state.focused_block == FocusedBlock::SessionsList {
                            vec![Span::styled(
                                " (e: edit)",
                                Style::default().fg(style::GRAY_DIM),
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

    let mut stateful = ListState::default();
    if !items.is_empty() {
        stateful.select(Some(state.selected.min(items.len() - 1)));
    }
    f.render_stateful_widget(list, list_area, &mut stateful);

    if let (Some(timer), Some(gauge_area)) = (&state.timer, gauge_area) {
        let pct = if timer.total == 0 {
            0.0
        } else {
            1.0 - (timer.remaining as f64 / timer.total as f64)
        };
        let ratio = pct.clamp(0.0, 1.0);
        let label = format!("{:.0}%", ratio * 100.0);

        let gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Timer Progress")
                    .style(dimmed)
                    .border_style(get_block_style(
                        state.focused_block,
                        FocusedBlock::SessionsList,
                        &state.mode,
                    )),
            )
            .gauge_style(Style::default().fg(style::BLUE))
            .ratio(ratio)
            .label(label)
            .use_unicode(true);

        f.render_widget(gauge, gauge_area);
    }

    // ── Notes panel ──
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
        let notes_inner = notes_block.inner(body_chunks[1]);
        f.render_widget(notes_block, body_chunks[1]);

        if matches!(state.mode, Mode::NotesEdit) {
            f.render_widget(&state.notes_textarea, notes_inner);
        } else {
            let notes_content = state.notes_textarea.lines().join("\n");
            let notes_para = Paragraph::new(notes_content).style(dimmed);
            f.render_widget(notes_para, notes_inner);
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

// ── Dialogs ──────────────────────────────────────────────────────────────

fn render_prompted_textarea_line(
    f: &mut ratatui::Frame,
    area: Rect,
    prompt: &str,
    textarea: &TextArea<'_>,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(prompt.len() as u16), Constraint::Min(1)])
        .split(area);
    f.render_widget(Paragraph::new(prompt), chunks[0]);
    f.render_widget(textarea, chunks[1]);
}

fn render_labeled_form_field(
    f: &mut ratatui::Frame,
    area: Rect,
    prefix: &str,
    style: Style,
    textarea: &TextArea<'_>,
    is_active: bool,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(prefix.len() as u16), Constraint::Min(1)])
        .split(area);
    f.render_widget(Paragraph::new(prefix).style(style), chunks[0]);
    if is_active {
        f.render_widget(textarea, chunks[1]);
    } else {
        f.render_widget(
            Paragraph::new(single_line_textarea_value(textarea)).style(style),
            chunks[1],
        );
    }
}

fn render_goal_selector_dialog(f: &mut ratatui::Frame, state: &AppState) {
    if !matches!(state.mode, Mode::AddSession | Mode::AddReward) {
        return;
    }

    let popup_area = centered_rect(80, 70, f.area());
    f.render_widget(ratatui::widgets::Clear, popup_area);

    let prompt = if matches!(state.mode, Mode::AddReward) {
        "Choose reward"
    } else {
        "Choose goal"
    };

    let popup_block = Block::default()
        .borders(Borders::ALL)
        .title(prompt)
        .border_style(Style::default().fg(style::BLUE));

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

    render_prompted_textarea_line(f, dialog_chunks[0], "> ", &state.search_input);

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
            .fg(style::BLUE)
            .add_modifier(Modifier::BOLD),
    );

    f.render_stateful_widget(list, dialog_chunks[1], &mut list_state);

    f.render_widget(
        Paragraph::new("Type to search • ↑↓ select • Enter pick • Esc cancel")
            .style(Style::default().fg(style::GRAY_DIM)),
        dialog_chunks[2],
    );
}

fn render_goal_form_dialog(f: &mut ratatui::Frame, state: &AppState) {
    let Some(form) = &state.form_state else {
        return;
    };

    let area = centered_rect(80, 70, f.area());
    f.render_widget(ratatui::widgets::Clear, area);

    let title = if form.is_reward {
        "Create new reward"
    } else {
        "Create new goal"
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(style::BLUE));

    let inner = block.inner(area);
    f.render_widget(block, area);

    #[cfg(feature = "web")]
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Goal Name
            Constraint::Length(1), // Quantity
            Constraint::Length(1), // Commands
            Constraint::Length(1), // Web note
            Constraint::Length(1), // Spacer
            Constraint::Min(1),    // Filler
            Constraint::Length(1), // Help text
        ])
        .split(inner);
    #[cfg(not(feature = "web"))]
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

    #[cfg(feature = "web")]
    {
        let web_note = Paragraph::new(
            "Commands are run when starting a session so that apps you used for a certain task are always opened — only available in full version",
        )
        .style(Style::default().fg(style::YELLOW));
        f.render_widget(web_note, layout[3]);
    }

    let name_prefix = "Name: ";
    let name_style = if form.current_field == FormField::GoalName {
        Style::default().fg(style::BLUE)
    } else {
        Style::default()
    };
    render_labeled_form_field(
        f,
        layout[0],
        name_prefix,
        name_style,
        &form.goal_name,
        form.current_field == FormField::GoalName,
    );

    let qty_prefix = "Quantity name (optional): ";
    let qty_style = if form.current_field == FormField::Quantity {
        Style::default().fg(style::BLUE)
    } else {
        Style::default()
    };
    render_labeled_form_field(
        f,
        layout[1],
        qty_prefix,
        qty_style,
        &form.quantity_name,
        form.current_field == FormField::Quantity,
    );

    let cmd_prefix = "Commands (optional, separated by ;): ";
    let cmd_style = if form.current_field == FormField::Commands {
        Style::default().fg(style::BLUE)
    } else {
        Style::default()
    };
    render_labeled_form_field(
        f,
        layout[2],
        cmd_prefix,
        cmd_style,
        &form.commands,
        form.current_field == FormField::Commands,
    );

    #[cfg(feature = "web")]
    let help_text =
        "↑↓/Tab: navigate • Enter: create • Esc: cancel • Note: Commands only work in full version";
    #[cfg(not(feature = "web"))]
    let help_text = "↑↓/Tab: navigate • Enter: create • Esc: cancel";
    let help = Paragraph::new(help_text).style(Style::default().fg(style::GRAY_DIM));
    #[cfg(feature = "web")]
    f.render_widget(help, layout[6]);
    #[cfg(not(feature = "web"))]
    f.render_widget(help, layout[5]);
}

fn render_duration_input_dialog(f: &mut ratatui::Frame, state: &AppState) {
    let Mode::DurationInput { ref goal_name, .. } = state.mode else {
        return;
    };

    let area = centered_rect_fixed_height(60, 4, f.area());
    f.render_widget(ratatui::widgets::Clear, area);

    let title = format!("Duration for {} (e.g., 30m, 1h)", goal_name);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(style::BLUE));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Input
            Constraint::Min(1),    // Help
        ])
        .split(inner);

    render_prompted_textarea_line(f, layout[0], "> ", &state.duration_input);

    f.render_widget(
        Paragraph::new("Enter: start • Esc: cancel").style(Style::default().fg(style::GRAY_DIM)),
        layout[1],
    );
}

fn render_quantity_input_dialog(f: &mut ratatui::Frame, state: &AppState) {
    let Mode::QuantityDoneInput {
        ref goal_name,
        ref quantity_name,
    } = state.mode
    else {
        return;
    };

    let area = centered_rect_fixed_height(60, 4, f.area());
    f.render_widget(ratatui::widgets::Clear, area);

    let title = if let Some(name) = quantity_name {
        format!("{} done for {} (blank to skip)", name, goal_name)
    } else {
        format!("Quantity done for {} (blank to skip)", goal_name)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(style::BLUE));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Input
            Constraint::Min(1),    // Help
        ])
        .split(inner);

    render_prompted_textarea_line(f, layout[0], "> ", &state.quantity_input);

    f.render_widget(
        Paragraph::new("Enter: confirm • Esc: skip").style(Style::default().fg(style::GRAY_DIM)),
        layout[1],
    );
}
