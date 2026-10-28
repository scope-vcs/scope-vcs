use crate::{
    api::{RequestChecksResponse, RequestReviewBotResponse, RequestReviewBotState},
    display::terminal_text,
};

const UNENFORCED: &str = "Required review bots do not hold the merge while the repository \
    opens no pull requests for review bots.";

pub(in crate::request) fn detail_lines(checks: &RequestChecksResponse) -> Vec<String> {
    let bots = &checks.review_bots;
    if bots.is_empty() {
        return Vec::new();
    }
    std::iter::once(format!(
        "  review bots: {}",
        bots.iter().map(bot_label).collect::<Vec<_>>().join(", ")
    ))
    .chain(unenforced(checks).map(|note| format!("  {note}")))
    .collect()
}

pub(in crate::request) fn checks_lines(checks: &RequestChecksResponse) -> Vec<String> {
    let bots = &checks.review_bots;
    if bots.is_empty() {
        return Vec::new();
    }
    std::iter::once("Review bots:".to_string())
        .chain(bots.iter().map(|bot| format!("  {}", bot_label(bot))))
        .chain(unenforced(checks).map(|note| format!("  {note}")))
        .collect()
}

fn unenforced(checks: &RequestChecksResponse) -> Option<&'static str> {
    (!checks.review_bots_enforced && checks.review_bots.iter().any(|bot| bot.required))
        .then_some(UNENFORCED)
}

fn bot_label(bot: &RequestReviewBotResponse) -> String {
    format!(
        "{}{} {}",
        terminal_text(&bot.login),
        if bot.required { " (required)" } else { "" },
        state_label(bot.state)
    )
}

fn state_label(state: RequestReviewBotState) -> &'static str {
    match state {
        RequestReviewBotState::NotStarted => "has not started on this head",
        RequestReviewBotState::Working => "is reviewing",
        RequestReviewBotState::Commented => "finished with comments",
        RequestReviewBotState::Clear => "finished with nothing found",
        RequestReviewBotState::TimedOut => "did not start in time and is advisory on this head",
    }
}
