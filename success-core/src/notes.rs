use crate::app::AppState;
use crate::utils::selected_goal_id;

pub fn refresh_notes_for_selection(state: &mut AppState) {
    if let Some(goal_id) = selected_goal_id(state) {
        state.notes =
            successlib::get_note(state.archive_path.clone(), goal_id).unwrap_or_default();
        state.notes_cursor = state.notes.len();
    } else {
        state.notes.clear();
        state.notes_cursor = 0;
    }
}

/// Save the notes for the currently selected goal.
pub fn save_notes_for_selection(state: &mut AppState) {
    if let Some(goal_id) = selected_goal_id(state) {
        let content = state.notes.clone();
        let _ = successlib::edit_note(state.archive_path.clone(), goal_id, content);
    }
}

pub fn notes_cursor_line_col(state: &AppState) -> (usize, usize) {
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
    if state.notes_cursor > seen {
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

pub fn line_starts(notes: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (idx, ch) in notes.char_indices() {
        if ch == '\n' {
            starts.push(idx + ch.len_utf8());
        }
    }
    starts
}

pub fn move_notes_cursor_left(state: &mut AppState) {
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

pub fn move_notes_cursor_right(state: &mut AppState) {
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

pub fn move_notes_cursor_word_left(state: &mut AppState) {
    if state.notes_cursor == 0 {
        return;
    }
    let mut idx = state.notes_cursor;
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

pub fn move_notes_cursor_word_right(state: &mut AppState) {
    let len = state.notes.len();
    let mut idx = state.notes_cursor;
    if idx >= len {
        return;
    }
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

pub fn move_notes_cursor_vert(state: &mut AppState, delta: isize) {
    let starts = line_starts(&state.notes);
    let (line, col) = notes_cursor_line_col(state);
    let new_line = line as isize + delta;
    if new_line < 0 || new_line as usize >= starts.len() {
        return;
    }
    let new_line = new_line as usize;
    let line_start = starts[new_line];
    let line_end = if new_line + 1 < starts.len() {
        starts[new_line + 1].saturating_sub(1)
    } else {
        state.notes.len()
    };
    let line_len = line_end.saturating_sub(line_start);
    let target_col = col.min(line_len);
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
