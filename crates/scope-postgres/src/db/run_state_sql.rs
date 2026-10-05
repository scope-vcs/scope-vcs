use scope_domain::runs::{attempt::AttemptState, job::RunJobState, run::RunState};

fn state_set(states: impl IntoIterator<Item = &'static str>) -> String {
    states
        .into_iter()
        .map(|state| format!("'{state}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn attempt_active_states() -> String {
    state_set(
        AttemptState::ALL
            .into_iter()
            .filter(|state| !state.is_terminal())
            .map(AttemptState::as_str),
    )
}

pub(super) fn attempt_terminal_states() -> String {
    state_set(
        AttemptState::ALL
            .into_iter()
            .filter(|state| state.is_terminal())
            .map(AttemptState::as_str),
    )
}

pub(super) fn run_active_states() -> String {
    state_set(
        RunState::ALL
            .into_iter()
            .filter(|state| !state.is_terminal())
            .map(RunState::as_str),
    )
}

pub(super) fn queued_job_state() -> String {
    state_set([RunJobState::Queued.as_str()])
}
