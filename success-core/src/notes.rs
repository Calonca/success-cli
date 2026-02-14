use crate::app::AppState;
use crate::utils::selected_goal_id;
use tui_textarea::{CursorMove, TextArea};

fn notes_to_textarea(notes: &str) -> TextArea<'static> {
    let mut textarea = TextArea::from(notes.split('\n'));
    textarea.set_tab_length(4);
    textarea.move_cursor(CursorMove::Bottom);
    textarea.move_cursor(CursorMove::End);
    textarea
}

pub fn refresh_notes_for_selection(state: &mut AppState) {
    if let Some(goal_id) = selected_goal_id(state) {
        let notes = successlib::get_note(state.archive_path.clone(), goal_id).unwrap_or_default();
        state.notes_textarea = notes_to_textarea(&notes);
    } else {
        state.notes_textarea = TextArea::default();
        state.notes_textarea.set_tab_length(4);
    }
}

/// Save the notes for the currently selected goal.
pub fn save_notes_for_selection(state: &mut AppState) {
    if let Some(goal_id) = selected_goal_id(state) {
        let content = state.notes_textarea.lines().join("\n");
        let _ = successlib::edit_note(state.archive_path.clone(), goal_id, content);
    }
}
