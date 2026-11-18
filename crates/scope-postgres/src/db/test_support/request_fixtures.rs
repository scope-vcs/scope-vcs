use crate::db::{RequestStore, RequestTransaction};
use scope_domain::requests::{
    CloseRequestInput, CloseRequestMutation, CreateRequestDiscussionInput,
    CreateRequestDiscussionMutation, CreateRequestDiscussionReplyInput,
    CreateRequestDiscussionReplyMutation, EditRequestIdentityInput, RequestDiscussion,
    RequestDiscussionTransitionInput, RequestLifecycleMutation, RequestTimelineMutation,
    SubmitRequestInput, close_request, create_request_discussion, create_request_discussion_reply,
    edit_request_identity, resolve_request_discussion, submit_request,
};

pub(crate) async fn submitted(
    store: &RequestStore,
    input: SubmitRequestInput,
) -> RequestLifecycleMutation {
    let tx = store
        .begin_request(&input.request_id, &input.actor_user_id)
        .await
        .unwrap();
    let now_unix = input.now_unix;
    let mutation = submit_request(tx.request(), tx.actor(), input).unwrap();
    tx.save_submission(&mutation, now_unix).await.unwrap();
    tx.commit().await.unwrap();
    mutation
}

pub(crate) async fn identity_edited(
    store: &RequestStore,
    input: EditRequestIdentityInput,
) -> RequestTimelineMutation {
    let tx = store
        .begin_request(&input.request_id, &input.actor_user_id)
        .await
        .unwrap();
    let now_unix = input.now_unix;
    let description = input.description_markdown.clone();
    let mutation = edit_request_identity(tx.request().clone(), tx.actor(), false, input).unwrap();
    tx.save_identity_edit(&mutation, description.as_deref(), now_unix)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    mutation
}

pub(crate) async fn closed(store: &RequestStore, input: CloseRequestInput) -> CloseRequestMutation {
    let tx = store
        .begin_request_close(&input.request_id, &input.actor_user_id)
        .await
        .unwrap();
    let facts = tx.close_facts().await.unwrap();
    let now_unix = input.now_unix;
    let mutation =
        close_request(tx.request().clone(), facts.events, facts.revisions, input).unwrap();
    tx.save_close(&mutation, None, now_unix).await.unwrap();
    tx.commit().await.unwrap();
    mutation
}

pub(crate) async fn discussion_created(
    store: &RequestStore,
    input: CreateRequestDiscussionInput,
) -> CreateRequestDiscussionMutation {
    let tx = store
        .begin_request(&input.request_id, &input.actor_user_id)
        .await
        .unwrap();
    let now_unix = input.now_unix;
    let mutation =
        create_request_discussion(tx.request().clone(), tx.actor(), false, input).unwrap();
    tx.save_created_discussion(&mutation, now_unix)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    mutation
}

pub(crate) async fn replied(
    store: &RequestStore,
    input: CreateRequestDiscussionReplyInput,
    wait_after_reply: bool,
) -> CreateRequestDiscussionReplyMutation {
    let tx = store
        .begin_request(&input.request_id, &input.actor_user_id)
        .await
        .unwrap();
    let discussion = discussion(&tx, &input.discussion_id).await;
    let quoted = match input.reply_to_reply_id.as_deref() {
        Some(quoted_id) => tx.reply(quoted_id).await.unwrap(),
        None => None,
    };
    let now_unix = input.now_unix;
    let mutation = create_request_discussion_reply(
        tx.request().clone(),
        tx.actor(),
        discussion,
        quoted.as_ref(),
        false,
        input,
    )
    .unwrap();
    tx.save_discussion_reply(&mutation, wait_after_reply, now_unix)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    mutation
}

pub(crate) async fn discussion_resolved(
    store: &RequestStore,
    input: RequestDiscussionTransitionInput,
) -> RequestDiscussion {
    let tx = store
        .begin_request(&input.request_id, &input.actor_user_id)
        .await
        .unwrap();
    let discussion = discussion(&tx, &input.discussion_id).await;
    let mutation =
        resolve_request_discussion(tx.request().clone(), tx.actor(), discussion, input).unwrap();
    tx.save_discussion_transition(&mutation).await.unwrap();
    tx.commit().await.unwrap();
    mutation.discussion
}

async fn discussion(tx: &RequestTransaction, discussion_id: &str) -> RequestDiscussion {
    tx.discussion(discussion_id).await.unwrap().unwrap()
}
