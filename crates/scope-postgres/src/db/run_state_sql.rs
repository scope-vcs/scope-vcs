//! SQL state sets built from the run domain enums, so predicates in this crate
//! cannot name a state the domain does not define or miss one it adds.

use scope_domain::runs::{attempt::AttemptState, job::RunJobState, run::RunState};

fn state_set(states: impl IntoIterator<Item = &'static str>) -> String {
    states
        .into_iter()
        .map(|state| format!("'{state}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Attempts a runner may still be executing.
pub(super) fn attempt_active_states() -> String {
    state_set(
        AttemptState::ALL
            .into_iter()
            .filter(|state| !state.is_terminal())
            .map(AttemptState::as_str),
    )
}

/// Attempts that have concluded and whose runner resources can be released.
pub(super) fn attempt_terminal_states() -> String {
    state_set(
        AttemptState::ALL
            .into_iter()
            .filter(|state| state.is_terminal())
            .map(AttemptState::as_str),
    )
}

/// Runs that can still dispatch work.
pub(super) fn run_active_states() -> String {
    state_set(
        RunState::ALL
            .into_iter()
            .filter(|state| !state.is_terminal())
            .map(RunState::as_str),
    )
}

/// The single job state that dispatch selects.
pub(super) fn queued_job_state() -> String {
    state_set([RunJobState::Queued.as_str()])
}
