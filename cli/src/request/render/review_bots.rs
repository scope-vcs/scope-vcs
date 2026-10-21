use crate::{
    api::{RequestReviewBotResponse, RequestReviewBotState},
    display::terminal_text,
};

pub(in crate::request) fn detail_line(bots: &[RequestReviewBotResponse]) -> Option<String> {
    (!bots.is_empty()).then(|| {
        format!(
            "  review bots: {}",
            bots.iter().map(bot_label).collect::<Vec<_>>().join(", ")
        )
    })
}

pub(in crate::request) fn checks_lines(bots: &[RequestReviewBotResponse]) -> Vec<String> {
    if bots.is_empty() {
        return Vec::new();
    }
    std::iter::once("Review bots:".to_string())
        .chain(bots.iter().map(|bot| format!("  {}", bot_label(bot))))
        .collect()
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
    }
}
