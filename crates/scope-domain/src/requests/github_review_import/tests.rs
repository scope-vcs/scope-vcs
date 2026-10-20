use super::*;
use crate::error::DomainErrorKind;
use crate::requests::fixtures::{open_request, source_blob};
use crate::requests::{RequestAttentionReason, RequestAttentionState, RequestBase};

fn revision(id: &str, position: u64, head: &str) -> RequestRevision {
    RequestRevision {
        id: id.to_string(),
        request_id: "request_1".to_string(),
        position,
        actor_user_id: Some("author".to_string()),
        old_head_oid: "base".to_string(),
        new_head_oid: head.to_string(),
        base: RequestBase::Main {
            review_base_oid: "base".to_string(),
        },
        base_version: 0,
        main_ancestor_oid: "base".to_string(),
        rewrote_history: false,
        git_snapshot: source_blob(head),
        created_at_unix: position,
    }
}

fn revisions() -> Vec<RequestRevision> {
    vec![
        revision("revision_1", 1, "old-head"),
        revision("revision_2", 2, "head"),
        revision("revision_3", 3, "head"),
    ]
}

fn line_position(
    side: RequestDiffSide,
    start_line: Option<u32>,
    line: Option<u32>,
) -> GitHubCommentPosition {
    GitHubCommentPosition {
        path: "src/lib.rs".to_string(),
        subject: GitHubCommentSubject::Line {
            side,
            start_line,
            line,
            original_start_line: Some(4),
            original_line: Some(6),
        },
        commit_oid: "head".to_string(),
        original_commit_oid: "old-head".to_string(),
    }
}

fn path() -> Option<ScopePath> {
    Some(ScopePath::parse("/src/lib.rs").unwrap())
}

fn lines(side: RequestDiffSide, start: u32, end: u32) -> Option<RequestDiscussionLines> {
    Some(RequestDiscussionLines {
        side,
        range: RequestLineRange::new(start, end).unwrap(),
    })
}

fn anchor(
    revision_id: &str,
    path: Option<ScopePath>,
    lines: Option<RequestDiscussionLines>,
) -> RequestDiscussionAnchor {
    RequestDiscussionAnchor {
        revision_id: revision_id.to_string(),
        commit_oid: None,
        path,
        lines,
    }
}

fn full_view_request() -> (Request, Views) {
    let views = Views::builtin();
    let mut request = open_request();
    request.view = views.full().clone();
    (request, views)
}

fn discussion_input(id: &str) -> ImportGitHubDiscussionInput {
    ImportGitHubDiscussionInput {
        id: id.to_string(),
        client_discussion_id: github_review_comment_client_id(11),
        github_account_id: 900,
        source_url: "https://github.com/octo/repo/pull/1#discussion_r11".to_string(),
        body_markdown: "Consider a guard here.".to_string(),
        anchor: None,
        created_at_unix: 50,
        now_unix: 60,
    }
}

#[test]
fn line_comments_anchor_to_the_newest_revision_with_that_head_on_their_side() {
    assert_eq!(
        github_comment_anchors(
            &line_position(RequestDiffSide::New, None, Some(8)),
            &revisions()
        ),
        vec![
            anchor("revision_3", path(), lines(RequestDiffSide::New, 8, 8)),
            anchor("revision_3", path(), None),
            anchor("revision_3", None, None),
        ]
    );
    assert_eq!(
        github_comment_anchors(
            &line_position(RequestDiffSide::Old, Some(2), Some(5)),
            &revisions()
        )[0],
        anchor("revision_3", path(), lines(RequestDiffSide::Old, 2, 5))
    );
}

#[test]
fn outdated_comments_use_their_original_commit_and_lines() {
    assert_eq!(
        github_comment_anchors(
            &line_position(RequestDiffSide::New, None, None),
            &revisions()
        )[0],
        anchor("revision_1", path(), lines(RequestDiffSide::New, 4, 6))
    );
}

#[test]
fn file_comments_and_unusable_lines_fall_back_to_file_then_revision() {
    let mut file = line_position(RequestDiffSide::New, None, None);
    file.subject = GitHubCommentSubject::File;
    assert_eq!(
        github_comment_anchors(&file, &revisions()),
        vec![
            anchor("revision_3", path(), None),
            anchor("revision_3", None, None)
        ]
    );
    assert_eq!(
        github_comment_anchors(
            &line_position(RequestDiffSide::New, Some(9), Some(3)),
            &revisions()
        ),
        vec![
            anchor("revision_3", path(), None),
            anchor("revision_3", None, None)
        ]
    );
    let mut root = line_position(RequestDiffSide::New, None, Some(1));
    root.path = "/".to_string();
    assert_eq!(
        github_comment_anchors(&root, &revisions()),
        vec![anchor("revision_3", None, None)]
    );
    let mut unknown = line_position(RequestDiffSide::New, None, Some(1));
    unknown.commit_oid = "elsewhere".to_string();
    assert!(github_comment_anchors(&unknown, &revisions()).is_empty());
}

#[test]
fn review_bodies_anchor_to_the_reviewed_revision_when_scope_knows_it() {
    assert_eq!(
        github_review_anchor(Some("old-head"), &revisions()),
        Some(anchor("revision_1", None, None))
    );
    assert_eq!(github_review_anchor(Some("elsewhere"), &revisions()), None);
    assert_eq!(github_review_anchor(None, &revisions()), None);
}

#[test]
fn comment_bodies_are_trimmed_and_capped_on_a_character_boundary() {
    assert_eq!(github_comment_body("  \n "), None);
    assert_eq!(github_comment_body(" ok ").as_deref(), Some("ok"));
    let long = "é".repeat(REQUEST_DISCUSSION_BODY_MAX_BYTES);
    let capped = github_comment_body(&long).unwrap();
    assert!(capped.len() <= REQUEST_DISCUSSION_BODY_MAX_BYTES);
    assert!(capped.len() >= REQUEST_DISCUSSION_BODY_MAX_BYTES - 1);
}

#[test]
fn only_open_full_view_requests_accept_github_reviews() {
    let (request, views) = full_view_request();
    assert!(accepts_github_reviews(&request, &views));
    let mut public = request.clone();
    public.view = crate::views::ViewId::public();
    assert!(!accepts_github_reviews(&public, &views));
    let mut draft = request.clone();
    draft.submitted_at_unix = None;
    assert!(!accepts_github_reviews(&draft, &views));
    let mut closed = request.clone();
    closed.closed_at_unix = Some(70);
    assert!(!accepts_github_reviews(&closed, &views));
    assert_eq!(
        import_github_discussion(&mut draft, &views, discussion_input("discussion_1"))
            .unwrap_err()
            .kind,
        DomainErrorKind::Conflict
    );
}

#[test]
fn imported_discussions_are_credited_to_the_github_account() {
    let (mut request, views) = full_view_request();
    let before = request.activity_version;
    let mut discussion =
        import_github_discussion(&mut request, &views, discussion_input("discussion_1")).unwrap();
    assert_eq!(
        discussion.author,
        RequestDiscussionAuthor::GitHub {
            github_account_id: 900,
            source_url: "https://github.com/octo/repo/pull/1#discussion_r11".to_string(),
        }
    );
    assert_eq!(discussion.author.user_id(), None);
    assert_eq!(discussion.opened_position, before + 1);
    assert_eq!(discussion.created_at_unix, 50);

    let reply = import_github_reply(
        &mut request,
        &views,
        &mut discussion,
        ImportGitHubReplyInput {
            id: "reply_1".to_string(),
            client_reply_id: github_review_comment_client_id(12),
            github_account_id: 901,
            source_url: "https://github.com/octo/repo/pull/1#discussion_r12".to_string(),
            body_markdown: "Done.".to_string(),
            created_at_unix: 55,
            now_unix: 61,
        },
    )
    .unwrap();
    assert_eq!(reply.position, before + 2);
    assert_eq!(discussion.last_activity_position, before + 2);
    assert_eq!(reply.author.github_account_id(), Some(901));

    let mut missing_link = discussion_input("discussion_2");
    missing_link.source_url = " ".to_string();
    assert_eq!(
        import_github_discussion(&mut request, &views, missing_link)
            .unwrap_err()
            .kind,
        DomainErrorKind::InvalidInput
    );
}

#[test]
fn github_thread_resolution_records_the_account_and_ignores_repeats() {
    let (mut request, views) = full_view_request();
    let mut discussion =
        import_github_discussion(&mut request, &views, discussion_input("discussion_1")).unwrap();
    assert!(
        transition_github_discussion(
            &mut request,
            &views,
            &mut discussion,
            902,
            RequestDiscussionStatus::Resolved,
            70
        )
        .unwrap()
    );
    assert_eq!(
        discussion.resolved_by,
        Some(RequestDiscussionResolver::GitHub(902))
    );
    assert_eq!(discussion.resolved_at_unix, Some(70));
    let position = discussion.last_activity_position;
    assert!(
        !transition_github_discussion(
            &mut request,
            &views,
            &mut discussion,
            902,
            RequestDiscussionStatus::Resolved,
            71
        )
        .unwrap()
    );
    assert_eq!(discussion.last_activity_position, position);
    assert!(
        transition_github_discussion(
            &mut request,
            &views,
            &mut discussion,
            902,
            RequestDiscussionStatus::Open,
            72
        )
        .unwrap()
    );
    assert_eq!(discussion.resolved_by, None);
    assert_eq!(discussion.status, RequestDiscussionStatus::Open);

    discussion.author = RequestDiscussionAuthor::User("author".to_string());
    assert_eq!(
        transition_github_discussion(
            &mut request,
            &views,
            &mut discussion,
            902,
            RequestDiscussionStatus::Resolved,
            73
        )
        .unwrap_err()
        .kind,
        DomainErrorKind::Forbidden
    );
}

#[test]
fn github_reviews_reactivate_only_the_request_authors_attention() {
    let (mut request, _) = full_view_request();
    request.activity_version = 9;
    let waiting = |user_id: &str| RequestAttention {
        request_id: request.id.clone(),
        user_id: user_id.to_string(),
        state: RequestAttentionState::Waiting,
        reason: RequestAttentionReason::Authored,
        through_activity_version: 4,
        snoozed_until_unix: None,
        updated_at_unix: 1,
        revision: 1,
    };
    let reactivated =
        reactivate_author_attention_for_github_review(&waiting("author"), &request, 80).unwrap();
    assert_eq!(reactivated.state, RequestAttentionState::Active);
    assert_eq!(reactivated.through_activity_version, 9);
    assert_eq!(
        reactivate_author_attention_for_github_review(&waiting("reviewer"), &request, 80),
        None
    );
}
