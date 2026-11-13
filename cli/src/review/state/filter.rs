use super::{ReviewInput, ReviewState, ReviewStateAction};

impl ReviewState {
    pub(super) fn handle_filter_input(&mut self, input: ReviewInput) -> ReviewStateAction {
        match input {
            ReviewInput::Escape => {
                self.editing_filter = false;
                self.message = "Filter closed".to_string();
            }
            ReviewInput::Backspace => {
                self.filter.pop();
                self.rebuild_visible_items();
            }
            ReviewInput::Char(value) => {
                self.filter.push(value);
                self.rebuild_visible_items();
            }
            ReviewInput::Quit => {
                self.filter.push('q');
                self.rebuild_visible_items();
            }
            ReviewInput::Up
            | ReviewInput::Down
            | ReviewInput::Left
            | ReviewInput::Right
            | ReviewInput::Toggle
            | ReviewInput::Save
            | ReviewInput::ContinuePush
            | ReviewInput::Filter
            | ReviewInput::Help
            | ReviewInput::Dependencies
            | ReviewInput::Cancel => {}
        }
        ReviewStateAction::None
    }
}
