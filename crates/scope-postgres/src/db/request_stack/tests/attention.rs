use super::*;
use crate::db::{ApplyRequestAttentionCommand, entities};
use scope_domain::requests::{
    RequestAttention, RequestAttentionAction, RequestAttentionReason, RequestAttentionState,
    RequestClaim,
};

async fn set_attention(
    store: &MetadataStore,
    request: &Request,
    action: RequestAttentionAction,
    now_unix: u64,
) {
    store
        .requests()
        .apply_request_attention(ApplyRequestAttentionCommand {
            repo_id: request.repo_id.clone(),
            request_id: request.id.clone(),
            actor_user_id: "user_owner".into(),
            expected_activity_version: request.activity_version,
            action,
            now_unix,
        })
        .await
        .unwrap();
}

async fn snapshot(
    store: &MetadataStore,
    request_id: &str,
) -> (RequestAttention, Option<RequestClaim>) {
    let attention = entities::request_attention_state::Entity::find_by_id((
        request_id.to_string(),
        "user_owner".to_string(),
    ))
    .one(store.db.as_ref())
    .await
    .unwrap()
    .unwrap()
    .try_into_domain()
    .unwrap();
    let claim = entities::request_claim::Entity::find_by_id(request_id)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .map(|claim| claim.try_into_domain().unwrap());
    (attention, claim)
}

#[tokio::test]
async fn dependencies_preserve_claims_and_future_snoozes_and_only_wake_on_hard_blockers() {
    let store = crate::db::requests::tests::postgres_store();
    let parent = start(
        &store,
        "parent",
        RequestBase::Main {
            review_base_oid: "base".into(),
        },
        2,
    )
    .await;
    let mut children = Vec::new();
    for (id, action) in [
        ("waiting", RequestAttentionAction::Wait),
        ("settled", RequestAttentionAction::Settle),
        ("future", RequestAttentionAction::Snooze { until_unix: 100 }),
        ("expired", RequestAttentionAction::Snooze { until_unix: 10 }),
        ("active", RequestAttentionAction::Claim),
    ] {
        let child = start(&store, id, pin(&store, &parent).await, 6).await;
        set_attention(&store, &child, RequestAttentionAction::Claim, 9).await;
        set_attention(&store, &child, action, 9).await;
        let before = snapshot(&store, id).await;
        children.push((child, before));
    }
    let publication = input(&parent, "extension", "parent_extension", 12);
    let facts = children
        .iter()
        .map(|(child, _)| fact(&publication.new_head_oid, child, true))
        .collect();
    let extended = store
        .requests()
        .record_request_revision_with_stack(publication, parent.base_version, None, facts, IDS)
        .await
        .unwrap()
        .request;
    for (child, before) in &children {
        assert_eq!(snapshot(&store, &child.id).await, *before);
        let refreshed = load(&store, &child.id).await;
        assert_eq!(refreshed.stack_status, RequestStackStatus::ParentExtended);
        assert_eq!(refreshed.activity_version, child.activity_version);
    }
    let mut publication = input(&extended, "rewrite", "parent_rewrite", 15);
    publication.git_facts.contains_old_head = false;
    let facts = children
        .iter()
        .map(|(child, _)| fact(&publication.new_head_oid, child, false))
        .collect();
    store
        .requests()
        .record_request_revision_with_stack(publication, extended.base_version, None, facts, IDS)
        .await
        .unwrap();
    for (child, (before, claim)) in children {
        let (after, retained_claim) = snapshot(&store, &child.id).await;
        assert_eq!(retained_claim, claim);
        let refreshed = load(&store, &child.id).await;
        assert_eq!(refreshed.stack_status, RequestStackStatus::ParentRewritten);
        if child.id == "future" || child.id == "active" {
            assert_eq!(after, before);
        } else {
            assert_eq!(after.state, RequestAttentionState::Active);
            assert_eq!(after.reason, RequestAttentionReason::NewActivity);
            assert_eq!(after.snoozed_until_unix, None);
            assert_eq!(after.through_activity_version, refreshed.activity_version);
            assert_eq!(after.revision, before.revision + 1);
        }
    }
}

pub(super) async fn seed_landing_attention(
    store: &MetadataStore,
    child: &Request,
    descendant: &Request,
) -> Vec<(String, (RequestAttention, Option<RequestClaim>))> {
    let mut snapshots = Vec::new();
    for (request, action) in [
        (child, RequestAttentionAction::Wait),
        (
            descendant,
            RequestAttentionAction::Snooze { until_unix: 100 },
        ),
    ] {
        set_attention(store, request, RequestAttentionAction::Claim, 13).await;
        set_attention(store, request, action, 13).await;
        snapshots.push((request.id.clone(), snapshot(store, &request.id).await));
    }
    snapshots
}

pub(super) async fn assert_landing_attention(
    store: &MetadataStore,
    snapshots: Vec<(String, (RequestAttention, Option<RequestClaim>))>,
) {
    for (id, expected) in snapshots {
        assert_eq!(snapshot(store, &id).await, expected);
    }
}
