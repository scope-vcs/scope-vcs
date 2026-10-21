use super::*;
use crate::github_review_bots::GitHubBotLogin;
use crate::requests::fixtures::{agent, views_with_agent};

const HEAD: &str = "1196d9ca5d1e0e7b8b1f6f0d6c2d3a4b5c6d7e8f";
const OLD: &str = "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567";
const CODEX: &str = "chatgpt-codex-connector[bot]";
const GREPTILE: &str = "greptile-apps[bot]";
const PUSHED_AT: u64 = 1_000;

fn bot(login: &str, required: bool) -> GitHubReviewBot {
    GitHubReviewBot {
        login: GitHubBotLogin::parse(login).unwrap(),
        required,
    }
}

fn codex_comment(code_review: &str, security_review: &str) -> String {
    format!(
        "<!-- codex-pull-request-review-summary -->\n\
         <!-- codex-review-state:v1 {{\"code_review\":{code_review},\"security_review\":{security_review}}} -->\n\
         <!-- codex-security-review:v1 {{\"blockingSeverityThreshold\":\"P0\",\"headSha\":\"{HEAD}\",\"mergeGateEnabled\":false,\"pullRequestNumber\":727,\"repository\":\"scope-vcs/scope-vcs\",\"status\":\"running\"}} -->\n\
         ## Codex Review Summary ..."
    )
}

fn part(commit_sha: &str, status: &str, findings: &str) -> String {
    format!(
        "{{\"commit_sha\":\"{commit_sha}\",\"has_any_findings\":{findings},\"has_unresolved_findings\":{findings},\"status\":\"{status}\"}}"
    )
}

fn state(signals: &[GitHubReviewBotSignal], login: &str) -> GitHubReviewBotState {
    github_review_bot_statuses(&[bot(login, true)], signals, HEAD, Some(PUSHED_AT))[0].state
}

fn reacted(login: &str, reaction: GitHubBotReaction, at_unix: u64) -> GitHubReviewBotSignal {
    GitHubReviewBotSignal::Reacted {
        login: login.to_string(),
        reaction,
        at_unix,
    }
}

fn commented(body: String) -> GitHubReviewBotSignal {
    GitHubReviewBotSignal::Commented {
        login: CODEX.to_string(),
        body,
    }
}

#[test]
fn codex_state_comment_is_done_only_when_every_review_completed_the_head() {
    let running = codex_comment(
        &part(HEAD, "running", "null"),
        &part(HEAD, "running", "null"),
    );
    assert_eq!(
        CodexReviewState::parse(&running).unwrap().state_for(HEAD),
        Some(GitHubReviewBotState::Working)
    );
    let half = codex_comment(
        &part(HEAD, "completed", "false"),
        &part(HEAD, "running", "null"),
    );
    assert_eq!(
        CodexReviewState::parse(&half).unwrap().state_for(HEAD),
        Some(GitHubReviewBotState::Working)
    );
    let clear = codex_comment(
        &part(HEAD, "completed", "false"),
        &part(HEAD, "completed", "false"),
    );
    assert_eq!(
        CodexReviewState::parse(&clear).unwrap().state_for(HEAD),
        Some(GitHubReviewBotState::Clear)
    );
    let found = codex_comment(
        &part(HEAD, "completed", "false"),
        &part(HEAD, "completed", "true"),
    );
    assert_eq!(
        CodexReviewState::parse(&found).unwrap().state_for(HEAD),
        Some(GitHubReviewBotState::Commented)
    );
    let older = codex_comment(
        &part(OLD, "completed", "false"),
        &part(OLD, "completed", "false"),
    );
    assert_eq!(
        CodexReviewState::parse(&older).unwrap().state_for(HEAD),
        None
    );
    let mixed = codex_comment(
        &part(HEAD, "completed", "false"),
        &part(OLD, "completed", "false"),
    );
    assert_eq!(
        CodexReviewState::parse(&mixed).unwrap().state_for(HEAD),
        Some(GitHubReviewBotState::Working)
    );
    let abbreviated = codex_comment(
        &part("1196D9CA", "completed", "false"),
        &part("1196d9ca", "completed", "false"),
    );
    assert_eq!(
        CodexReviewState::parse(&abbreviated)
            .unwrap()
            .state_for(HEAD),
        Some(GitHubReviewBotState::Clear)
    );
    let code_only = "<!-- codex-review-state:v1 {\"code_review\":".to_string()
        + &part(HEAD, "completed", "false")
        + "} -->";
    assert_eq!(
        CodexReviewState::parse(&code_only).unwrap().state_for(HEAD),
        Some(GitHubReviewBotState::Clear)
    );
}

#[test]
fn malformed_codex_state_comments_report_nothing() {
    assert!(CodexReviewState::appears_in(&codex_comment("{}", "{}")));
    assert!(!CodexReviewState::appears_in(
        "<!-- codex-pull-request-review-summary -->"
    ));
    for body in [
        String::new(),
        "## Codex Review Summary".to_string(),
        "<!-- codex-review-state:v1 {\"code_review\":".to_string(),
        "<!-- codex-review-state:v1 not json -->".to_string(),
        "<!-- codex-review-state:v1 [] -->".to_string(),
        "<!-- codex-review-state:v1 {} -->".to_string(),
        "<!-- codex-review-state:v1 {\"code_review\":{\"status\":\"completed\"}} -->".to_string(),
        "<!-- codex-review-state:v1 {\"code_review\":\"done\"} -->".to_string(),
        "<!-- codex-review-state:v2 {\"code_review\":".to_string()
            + &part(HEAD, "completed", "false")
            + "} -->",
    ] {
        assert_eq!(CodexReviewState::parse(&body), None, "{body}");
    }
    let short = codex_comment(
        &part("1196d9", "completed", "false"),
        &part("not-a-sha", "completed", "false"),
    );
    assert_eq!(
        CodexReviewState::parse(&short).unwrap().state_for(HEAD),
        None
    );
}

#[test]
fn a_bot_is_done_after_reviewing_the_head_or_approving_after_it_was_pushed() {
    let reviewed = |commit_oid: &str| GitHubReviewBotSignal::Reviewed {
        login: "Greptile-Apps[bot]".to_string(),
        commit_oid: commit_oid.to_string(),
    };
    assert_eq!(state(&[], GREPTILE), GitHubReviewBotState::NotStarted);
    assert_eq!(
        state(&[reacted(GREPTILE, GitHubBotReaction::Eyes, 1)], GREPTILE),
        GitHubReviewBotState::Working
    );
    assert_eq!(
        state(&[reviewed(OLD)], GREPTILE),
        GitHubReviewBotState::NotStarted
    );
    assert_eq!(
        state(
            &[
                reviewed(HEAD),
                reacted(GREPTILE, GitHubBotReaction::Eyes, 1)
            ],
            GREPTILE
        ),
        GitHubReviewBotState::Commented
    );
    assert_eq!(
        state(
            &[reacted(GREPTILE, GitHubBotReaction::ThumbsUp, PUSHED_AT)],
            GREPTILE
        ),
        GitHubReviewBotState::Clear
    );
    assert_eq!(
        state(
            &[reacted(
                GREPTILE,
                GitHubBotReaction::ThumbsUp,
                PUSHED_AT - 1
            )],
            GREPTILE
        ),
        GitHubReviewBotState::NotStarted
    );
    assert_eq!(
        github_review_bot_statuses(
            &[bot(GREPTILE, true)],
            &[reacted(GREPTILE, GitHubBotReaction::ThumbsUp, PUSHED_AT)],
            HEAD,
            None
        )[0]
        .state,
        GitHubReviewBotState::NotStarted
    );
    assert_eq!(
        state(
            &[
                reacted("octo-reviewer[bot]", GitHubBotReaction::ThumbsUp, PUSHED_AT),
                reacted(GREPTILE, GitHubBotReaction::Eyes, 1),
            ],
            GREPTILE
        ),
        GitHubReviewBotState::Working
    );
}

#[test]
fn a_bot_with_a_state_comment_is_done_only_when_the_comment_says_so() {
    let thumbs_up = reacted(CODEX, GitHubBotReaction::ThumbsUp, PUSHED_AT + 1);
    let eyes = reacted(CODEX, GitHubBotReaction::Eyes, PUSHED_AT + 1);
    let older = commented(codex_comment(
        &part(OLD, "completed", "false"),
        &part(OLD, "completed", "false"),
    ));
    assert_eq!(
        state(&[older.clone(), thumbs_up.clone()], CODEX),
        GitHubReviewBotState::NotStarted
    );
    assert_eq!(
        state(&[older, eyes.clone()], CODEX),
        GitHubReviewBotState::Working
    );
    let running = commented(codex_comment(
        &part(HEAD, "completed", "false"),
        &part(HEAD, "running", "null"),
    ));
    assert_eq!(
        state(&[running, thumbs_up.clone()], CODEX),
        GitHubReviewBotState::Working
    );
    let clear = commented(codex_comment(
        &part(HEAD, "completed", "false"),
        &part(HEAD, "completed", "false"),
    ));
    assert_eq!(state(&[clear, eyes], CODEX), GitHubReviewBotState::Clear);
    let malformed = commented("<!-- codex-review-state:v1 nope -->".to_string());
    assert_eq!(
        state(&[malformed, thumbs_up], CODEX),
        GitHubReviewBotState::Clear
    );
}

fn reviews(
    bots: Vec<GitHubReviewBot>,
    states: &[(&str, GitHubReviewBotState)],
) -> RequestBotReviews {
    RequestBotReviews {
        bots,
        statuses: states
            .iter()
            .map(|(login, state)| GitHubReviewBotStatus {
                login: login.to_string(),
                head_oid: HEAD.to_string(),
                state: *state,
            })
            .collect(),
        open_finding_authors: Vec::new(),
    }
}

#[test]
fn required_bots_gate_full_view_requests_on_the_current_head_and_open_findings() {
    let views = views_with_agent();
    let full = views.full().clone();
    let outcome =
        |reviews: &RequestBotReviews| request_reviews_outcome(&full, HEAD, &views, reviews);
    use GitHubReviewBotState::*;

    let optional = reviews(vec![bot(GREPTILE, false)], &[]);
    assert_eq!(outcome(&optional), RequestReviewsOutcome::Clear);
    let optional_findings = RequestBotReviews {
        open_finding_authors: vec![GREPTILE.to_string()],
        ..optional
    };
    assert_eq!(outcome(&optional_findings), RequestReviewsOutcome::Clear);

    let bots = || vec![bot(CODEX, true), bot(GREPTILE, false)];
    assert_eq!(
        outcome(&reviews(bots(), &[(GREPTILE, Clear)])),
        RequestReviewsOutcome::Pending
    );
    assert_eq!(
        outcome(&reviews(bots(), &[(CODEX, Working)])),
        RequestReviewsOutcome::Pending
    );
    assert_eq!(
        outcome(&reviews(bots(), &[(CODEX, Commented)])),
        RequestReviewsOutcome::Clear
    );
    assert_eq!(
        outcome(&reviews(bots(), &[(CODEX, Clear)])),
        RequestReviewsOutcome::Clear
    );
    let stale = reviews(bots(), &[(CODEX, Clear)]);
    assert_eq!(
        request_reviews_outcome(&full, OLD, &views, &stale),
        RequestReviewsOutcome::Pending
    );

    let findings = |author: &str| RequestBotReviews {
        open_finding_authors: vec![author.to_string()],
        ..reviews(bots(), &[(CODEX, Clear)])
    };
    assert_eq!(
        outcome(&findings("GREPTILE-APPS[bot]")),
        RequestReviewsOutcome::FindingsOpen
    );
    assert_eq!(outcome(&findings("octocat")), RequestReviewsOutcome::Clear);
    assert_eq!(
        request_reviews_outcome(&agent(), HEAD, &views, &findings(GREPTILE)),
        RequestReviewsOutcome::Clear
    );
}
