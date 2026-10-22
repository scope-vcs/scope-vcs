use super::request_discussions::{CONTRIBUTOR_ID, MAINTAINER_ID};
use crate::error::ApiError;
use scope_domain::requests::RequestDiscussionAnchor;
use scope_postgres::db::{
    CreateRequestDiscussionCommand, CreateRequestDiscussionReplyCommand, DiscussionTransition,
    MetadataStore, TransitionRequestDiscussionCommand,
};

const REQUEST_ID: &str = "req_demo_ready";
const RETRY_CAP_ID: &str = "discussion_demo_retry_cap";
const RETRY_CAP_MAINTAINER_REPLY_ID: &str = "discussion_reply_demo_retry_cap_maintainer";
const RETRY_CAP_CONTRIBUTOR_REPLY_ID: &str = "discussion_reply_demo_retry_cap_quote";
const RESOLVED_DOCS_ID: &str = "discussion_demo_resolved_docs";
const JITTER_ID: &str = "discussion_demo_jitter";
const JITTER_REVISION_DISCUSSION_ID: &str = "discussion_demo_revision_jitter";
const TEST_REVISION_DISCUSSION_ID: &str = "discussion_demo_revision_tests";
const FINAL_REVISION_DISCUSSION_ID: &str = "discussion_demo_revision_final";

struct SeedRevisionDiscussion {
    discussion_id: &'static str,
    revision_id: &'static str,
    actor_user_id: &'static str,
    client_discussion_id: &'static str,
    body: &'static str,
}

pub(crate) async fn seed_request_discussion_gallery(
    metadata: &MetadataStore,
) -> Result<(), ApiError> {
    create_retry_cap_conversation(metadata).await?;
    create_jitter_conversation(metadata).await?;
    create_resolved_docs_conversation(metadata).await?;
    create_revision_conversations(metadata).await?;

    Ok(())
}

async fn create_revision_conversations(metadata: &MetadataStore) -> Result<(), ApiError> {
    let discussions = [
        SeedRevisionDiscussion {
            discussion_id: JITTER_REVISION_DISCUSSION_ID,
            revision_id: "event_req_demo_ready_revision_2",
            actor_user_id: MAINTAINER_ID,
            client_discussion_id: "seed_revision_jitter",
            body: concat!(
                "The bounded jitter looks right. A deterministic random source in the tests ",
                "would make the boundary behavior easy to review."
            ),
        },
        SeedRevisionDiscussion {
            discussion_id: TEST_REVISION_DISCUSSION_ID,
            revision_id: "event_req_demo_ready_revision_3",
            actor_user_id: CONTRIBUTOR_ID,
            client_discussion_id: "seed_revision_tests",
            body: "The tests now pin both the zero-jitter and maximum-jitter edges.",
        },
        SeedRevisionDiscussion {
            discussion_id: FINAL_REVISION_DISCUSSION_ID,
            revision_id: "event_req_demo_ready_revision_4",
            actor_user_id: MAINTAINER_ID,
            client_discussion_id: "seed_revision_final",
            body: concat!(
                "This closes the loop nicely: exported policy, implementation, tests, and ",
                "contributor documentation all agree."
            ),
        },
    ];
    for (index, discussion) in discussions.into_iter().enumerate() {
        metadata
            .requests()
            .create_request_discussion(CreateRequestDiscussionCommand {
                request_id: REQUEST_ID.to_string(),
                id: discussion.discussion_id.to_string(),
                actor_user_id: discussion.actor_user_id.to_string(),
                client_discussion_id: discussion.client_discussion_id.to_string(),
                body_markdown: discussion.body.to_string(),
                anchor: Some(RequestDiscussionAnchor {
                    revision_id: discussion.revision_id.to_string(),
                    commit_oid: None,
                    path: None,
                    lines: None,
                }),
                now_unix: 1_800_000_150 + index as u64,
            })
            .await?;
    }
    Ok(())
}

async fn create_retry_cap_conversation(metadata: &MetadataStore) -> Result<(), ApiError> {
    metadata
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: REQUEST_ID.to_string(),
            id: RETRY_CAP_ID.to_string(),
            actor_user_id: CONTRIBUTOR_ID.to_string(),
            client_discussion_id: "seed_retry_cap".to_string(),
            body_markdown: concat!(
                "Should the retry cap remain **2 seconds**, or should callers be able to ",
                "override it? I like the predictable default, but an explicit constant might ",
                "make the policy easier to discover."
            )
            .to_string(),
            anchor: None,
            now_unix: 1_800_000_120,
        })
        .await?;
    metadata
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            request_id: REQUEST_ID.to_string(),
            discussion_id: RETRY_CAP_ID.to_string(),
            id: RETRY_CAP_MAINTAINER_REPLY_ID.to_string(),
            actor_user_id: MAINTAINER_ID.to_string(),
            client_reply_id: "seed_retry_cap_maintainer".to_string(),
            body_markdown: concat!(
                "Two seconds is intentional for interactive commands. Let's extract ",
                "`MAX_RETRY_DELAY_MS` and leave per-command overrides out of this request."
            )
            .to_string(),
            reply_to_reply_id: None,
            wait_after_reply: false,
            now_unix: 1_800_000_121,
        })
        .await?;
    metadata
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            request_id: REQUEST_ID.to_string(),
            discussion_id: RETRY_CAP_ID.to_string(),
            id: RETRY_CAP_CONTRIBUTOR_REPLY_ID.to_string(),
            actor_user_id: CONTRIBUTOR_ID.to_string(),
            client_reply_id: "seed_retry_cap_quote".to_string(),
            body_markdown: concat!(
                "Agreed. Quoting the maintainer response here so the decision remains ",
                "attached to the suggestion: keep the fixed cap and name the constant."
            )
            .to_string(),
            reply_to_reply_id: Some(RETRY_CAP_MAINTAINER_REPLY_ID.to_string()),
            wait_after_reply: false,
            now_unix: 1_800_000_122,
        })
        .await?;
    metadata
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            request_id: REQUEST_ID.to_string(),
            discussion_id: RETRY_CAP_ID.to_string(),
            id: "discussion_reply_demo_retry_cap_nested".to_string(),
            actor_user_id: MAINTAINER_ID.to_string(),
            client_reply_id: "seed_retry_cap_nested".to_string(),
            body_markdown: concat!(
                "Exactly. Keeping that decision nested here makes the implementation history ",
                "easy to follow without expanding the whole conversation."
            )
            .to_string(),
            reply_to_reply_id: Some(RETRY_CAP_CONTRIBUTOR_REPLY_ID.to_string()),
            wait_after_reply: false,
            now_unix: 1_800_000_123,
        })
        .await?;
    Ok(())
}

async fn create_jitter_conversation(metadata: &MetadataStore) -> Result<(), ApiError> {
    metadata
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: REQUEST_ID.to_string(),
            id: JITTER_ID.to_string(),
            actor_user_id: MAINTAINER_ID.to_string(),
            client_discussion_id: "seed_jitter".to_string(),
            body_markdown: concat!(
                "Can we add a small amount of jitter before this lands? Simultaneous clients ",
                "currently retry on exactly the same boundaries."
            )
            .to_string(),
            anchor: None,
            now_unix: 1_800_000_130,
        })
        .await?;
    metadata
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            request_id: REQUEST_ID.to_string(),
            discussion_id: JITTER_ID.to_string(),
            id: "discussion_reply_demo_jitter".to_string(),
            actor_user_id: CONTRIBUTOR_ID.to_string(),
            client_reply_id: "seed_jitter_reply".to_string(),
            body_markdown: concat!(
                "Yes. I'll use bounded positive jitter and add a deterministic unit test ",
                "around the range rather than snapshotting random values."
            )
            .to_string(),
            reply_to_reply_id: None,
            wait_after_reply: false,
            now_unix: 1_800_000_131,
        })
        .await?;
    Ok(())
}

async fn create_resolved_docs_conversation(metadata: &MetadataStore) -> Result<(), ApiError> {
    metadata
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: REQUEST_ID.to_string(),
            id: RESOLVED_DOCS_ID.to_string(),
            actor_user_id: CONTRIBUTOR_ID.to_string(),
            client_discussion_id: "seed_resolved_docs".to_string(),
            body_markdown: concat!(
                "The helper accepts milliseconds, but the name `retryDelay` does not state ",
                "the unit. Could the doc comment make that explicit?"
            )
            .to_string(),
            anchor: None,
            now_unix: 1_800_000_140,
        })
        .await?;
    metadata
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            request_id: REQUEST_ID.to_string(),
            discussion_id: RESOLVED_DOCS_ID.to_string(),
            id: "discussion_reply_demo_resolved_docs".to_string(),
            actor_user_id: MAINTAINER_ID.to_string(),
            client_reply_id: "seed_resolved_docs_reply".to_string(),
            body_markdown: "The new doc comment now says the returned delay is in milliseconds."
                .to_string(),
            reply_to_reply_id: None,
            wait_after_reply: false,
            now_unix: 1_800_000_141,
        })
        .await?;
    metadata
        .requests()
        .transition_request_discussion(TransitionRequestDiscussionCommand {
            request_id: REQUEST_ID.to_string(),
            discussion_id: RESOLVED_DOCS_ID.to_string(),
            actor_user_id: MAINTAINER_ID.to_string(),
            event_id: "event_demo_discussion_resolved".to_string(),
            now_unix: 1_800_000_142,
            transition: DiscussionTransition::Resolve,
        })
        .await?;
    Ok(())
}
