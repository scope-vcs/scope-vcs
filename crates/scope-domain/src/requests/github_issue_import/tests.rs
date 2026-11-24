use super::*;
use crate::{github::account::GitHubAccountType, requests::RequestDiscussionAuthor};

const ISSUE_URL: &str = "https://github.com/octo/app/issues/142";

fn account(id: u64, login: &str) -> GitHubAccount {
    GitHubAccount::new(id, login, GitHubAccountType::User, None).unwrap()
}

fn issue() -> GitHubIssue {
    GitHubIssue {
        number: 142,
        title: "  Login times out on slow connections ".to_string(),
        body: "Sign-in gives up before the server answers.".to_string(),
        author: account(7, "tomasz-k"),
        source_url: ISSUE_URL.to_string(),
        created_at_unix: Some(100),
    }
}

fn comment(id: u64, author: GitHubAccount, body: &str, created_at_unix: u64) -> GitHubIssueComment {
    GitHubIssueComment {
        id,
        author,
        body: body.to_string(),
        source_url: format!("{ISSUE_URL}#issuecomment-{id}"),
        created_at_unix: Some(created_at_unix),
    }
}

fn import(issue: GitHubIssue, comments: Vec<GitHubIssueComment>) -> GitHubIssueImportResult {
    import_github_issue(
        OpenRequestFromAskFacts::default(),
        issue,
        comments,
        ImportGitHubIssueInput {
            request_id: "req_1".to_string(),
            repo_id: "owner/repo".to_string(),
            view: ViewId::private(),
            main_oid: "main".to_string(),
            started_event_id: "event_started".to_string(),
            discussion_ids: (1..=4).map(|index| format!("discussion_{index}")).collect(),
            now_unix: 500,
        },
        &Views::builtin(),
    )
}

fn imported(result: GitHubIssueImportResult) -> ImportedGitHubIssue {
    let GitHubIssueImportResult::Imported(imported) = result else {
        panic!("expected an imported issue, got {result:?}");
    };
    *imported
}

#[test]
fn an_issue_becomes_a_request_asked_by_its_author_with_its_comments_oldest_first() {
    let ines = account(9, "ines-r");
    let imported = imported(import(
        issue(),
        vec![
            comment(32, ines.clone(), " Raising the timeout fixes it. ", 300),
            comment(31, account(7, "tomasz-k"), "Still happens on 2.14.1.", 200),
            comment(32, ines.clone(), "Raising the timeout fixes it.", 300),
            comment(33, ines.clone(), "  \n", 400),
        ],
    ));

    let request = &imported.request;
    assert_eq!(request.title, "Login times out on slow connections");
    assert_eq!(request.name, "gh-142");
    assert_eq!(request.view, ViewId::private());
    assert_eq!(request.author_user_id, None);
    assert_eq!(request.created_at_unix, 500);
    assert_eq!(
        request.ask,
        Some(RequestAsk {
            github_account_id: 7,
            github_issue_number: 142,
            source_url: ISSUE_URL.to_string(),
            text_markdown: "Sign-in gives up before the server answers.".to_string(),
            asked_at_unix: 100,
        })
    );
    assert_eq!(imported.started.request_id, "req_1");

    let discussions = imported
        .discussions
        .iter()
        .map(|discussion| {
            (
                discussion.id.as_str(),
                discussion.client_discussion_id.as_str(),
                discussion.author.clone(),
                discussion.body_markdown.as_str(),
                discussion.created_at_unix,
                discussion.anchor.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        discussions,
        [
            (
                "discussion_1",
                "github:issue-comment:31",
                RequestDiscussionAuthor::GitHub {
                    github_account_id: 7,
                    source_url: format!("{ISSUE_URL}#issuecomment-31"),
                },
                "Still happens on 2.14.1.",
                200,
                None,
            ),
            (
                "discussion_2",
                "github:issue-comment:32",
                RequestDiscussionAuthor::GitHub {
                    github_account_id: 9,
                    source_url: format!("{ISSUE_URL}#issuecomment-32"),
                },
                "Raising the timeout fixes it.",
                300,
                None,
            ),
        ]
    );
    assert!(
        imported.discussions[0].opened_position < imported.discussions[1].opened_position,
        "the oldest comment opens first"
    );
    assert_eq!(
        request.activity_version,
        imported.discussions[1].opened_position
    );
    assert_eq!(imported.accounts, [account(7, "tomasz-k"), ines]);
}

#[test]
fn an_issue_that_already_has_a_request_is_left_alone() {
    let result = import_github_issue(
        OpenRequestFromAskFacts {
            github_issue_has_request: true,
            ..OpenRequestFromAskFacts::default()
        },
        GitHubIssue {
            title: String::new(),
            ..issue()
        },
        Vec::new(),
        ImportGitHubIssueInput {
            request_id: "req_2".to_string(),
            repo_id: "owner/repo".to_string(),
            view: ViewId::private(),
            main_oid: "main".to_string(),
            started_event_id: "event_started".to_string(),
            discussion_ids: Vec::new(),
            now_unix: 500,
        },
        &Views::builtin(),
    );
    assert_eq!(result, GitHubIssueImportResult::AlreadyImported);
    assert_eq!(result.step(), GitHubIssueImportStep::AlreadyImported);
}

#[test]
fn a_long_title_is_cut_on_a_character_boundary_and_ends_with_an_ellipsis() {
    let exact = "é".repeat(REQUEST_TITLE_MAX_BYTES / 2);
    assert_eq!(
        imported(import(
            GitHubIssue {
                title: exact.clone(),
                ..issue()
            },
            Vec::new()
        ))
        .request
        .title,
        exact
    );

    let title = imported(import(
        GitHubIssue {
            title: format!("a{exact}"),
            ..issue()
        },
        Vec::new(),
    ))
    .request
    .title;
    assert_eq!(title, format!("a{}…", "é".repeat(126)));
    assert!(title.len() <= REQUEST_TITLE_MAX_BYTES);
}

#[test]
fn text_over_a_cap_is_cut_and_ends_with_a_line_linking_the_original() {
    let notice =
        |url: &str| format!("\n\n_Truncated on import. [Read the original on GitHub]({url})._");
    let long_comment = comment(
        31,
        account(9, "ines-r"),
        &"x".repeat(REQUEST_DISCUSSION_BODY_MAX_BYTES + 1),
        200,
    );
    let imported = imported(import(
        GitHubIssue {
            body: "é".repeat(REQUEST_ASK_TEXT_MAX_BYTES / 2 + 1),
            ..issue()
        },
        vec![
            long_comment.clone(),
            comment(
                32,
                account(9, "ines-r"),
                &"x".repeat(REQUEST_DISCUSSION_BODY_MAX_BYTES),
                300,
            ),
        ],
    ));

    let ask = imported.request.ask.unwrap().text_markdown;
    let kept = ask.strip_suffix(&notice(ISSUE_URL)).unwrap();
    assert!(ask.len() <= REQUEST_ASK_TEXT_MAX_BYTES);
    assert!(ask.len() > REQUEST_ASK_TEXT_MAX_BYTES - 'é'.len_utf8());
    assert!(kept.chars().all(|character| character == 'é'));

    let cut = &imported.discussions[0].body_markdown;
    assert_eq!(cut.len(), REQUEST_DISCUSSION_BODY_MAX_BYTES);
    assert!(cut.ends_with(&notice(&long_comment.source_url)));
    assert_eq!(
        imported.discussions[1].body_markdown,
        "x".repeat(REQUEST_DISCUSSION_BODY_MAX_BYTES)
    );
}

#[test]
fn an_issue_scope_cannot_hold_is_skipped_with_its_reason() {
    for unimportable in [
        GitHubIssue {
            title: " \n".to_string(),
            ..issue()
        },
        GitHubIssue {
            created_at_unix: None,
            ..issue()
        },
        GitHubIssue {
            source_url: String::new(),
            ..issue()
        },
    ] {
        let result = import(unimportable.clone(), Vec::new());
        assert!(
            matches!(&result, GitHubIssueImportResult::Skipped { reason } if !reason.is_empty()),
            "{unimportable:?} gave {result:?}"
        );
        assert_eq!(result.step(), GitHubIssueImportStep::Skipped);
    }
}
