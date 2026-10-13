use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    git::request_refs::{
        create_request_receive_pack_staging_repo, request_revision_git_facts,
        with_request_revision_store_repo,
    },
    persistence::unix_now,
    persistence_ids::generate_prefixed_id,
    state::AppState,
    use_cases::request_base_publication::{
        RequestBasePublicationHeaders, prepare_request_base_publication,
        request_stack_child_publication_facts, restore_current_main,
    },
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_api_contract::{RequestRevisionContextRequest, RequestRevisionContextResponse};
use scope_domain::requests::{RecordRequestRevisionInput, RequestBase};
use scope_postgres::db::RequestBasePublicationFacts;
use scope_postgres::db::RequestRevisionContextPublication;
use scope_product_analytics::ProductEvent;

pub(crate) async fn publish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<RequestRevisionContextRequest>,
) -> Result<Json<RequestRevisionContextResponse>, ApiError> {
    let actor = require_scope_user(&state, &headers).await?;
    let base = match input.base {
        scope_api_contract::RequestBase::Request {
            request_id,
            revision_id,
            head_oid,
        } => RequestBase::Request {
            request_id,
            revision_id,
            head_oid: head_oid.as_str().to_string(),
        },
        scope_api_contract::RequestBase::Main { review_base_oid } => RequestBase::Main {
            review_base_oid: review_base_oid.as_str().to_string(),
        },
    };
    let context = RequestRevisionContextPublication {
        operation_key: input.operation_key,
        expected_revision_id: input.expected_revision_id,
        expected_head_oid: input.expected_head_oid.as_str().to_string(),
        expected_base_version: input.expected_base_version,
        base: base.clone(),
        expected_parent_base_version: input.expected_parent_base_version,
    };
    let snapshot =
        super::request_state::load(&state, &headers, &owner, &repo_name, &request_id).await?;
    let request = &snapshot.request;
    let incarnation = snapshot.repository.incarnation();
    let event_id = generate_prefixed_id("event_request_revision_context")?;
    let mutation = if let Some(receipt) = state
        .metadata
        .requests()
        .request_revision_context_receipt(&request_id, &actor.id, &context)
        .await?
    {
        receipt
    } else {
        let revision = snapshot
            .revision
            .as_ref()
            .ok_or_else(|| ApiError::conflict("request has no saved revision"))?;
        let staging_state = state.clone();
        let staging_incarnation = incarnation.clone();
        let staging = with_request_revision_store_repo(
            &state,
            &incarnation,
            request,
            revision,
            move |raw_repo, _| {
                create_request_receive_pack_staging_repo(
                    &staging_state,
                    &staging_incarnation,
                    raw_repo,
                )
            },
        )
        .await?;
        let publication = async {
            let publication_headers = RequestBasePublicationHeaders {
                base: base.clone(),
                expected_base_version: context.expected_base_version,
                expected_parent_base_version: context.expected_parent_base_version,
            };
            let (_, parent) = prepare_request_base_publication(
                &state,
                &incarnation,
                request,
                Some(publication_headers),
                &staging,
                &request.head_oid,
            )
            .await?;
            let main = match &parent {
                Some(RequestBasePublicationFacts::Main { main_head_oid, .. }) => {
                    main_head_oid.clone()
                }
                _ => restore_current_main(&state, &incarnation, &staging).await?,
            };
            let path = staging.clone();
            let old_main = request.main_ancestor_oid.clone();
            let head = request.head_oid.clone();
            let git_facts = crate::git::blocking::run(move || {
                request_revision_git_facts(&path, &old_main, &head, &head, Some(&main))
            })
            .await?;
            let children = request_stack_child_publication_facts(
                &state,
                &incarnation,
                request,
                &staging,
                &request.head_oid,
            )
            .await?;
            state
                .metadata
                .requests()
                .record_request_revision_context(
                    RecordRequestRevisionInput {
                        request_id: request.id.clone(),
                        actor_user_id: actor.id.clone(),
                        actor_can_edit: false,
                        expected_old_head_oid: Some(context.expected_head_oid.clone()),
                        new_head_oid: context.expected_head_oid.clone(),
                        new_base: Some(base),
                        new_base_stack_depth: None,
                        git_snapshot: revision.git_snapshot.clone(),
                        git_facts,
                        event_id: event_id.clone(),
                        body: None,
                        now_unix: unix_now()?,
                    },
                    context,
                    parent,
                    children,
                    &crate::persistence_ids::generate_persistence_id,
                )
                .await
                .map_err(ApiError::from)
        }
        .await;
        let _ = std::fs::remove_dir_all(staging);
        publication?
    };
    if mutation.event.id == event_id {
        state
            .product_analytics
            .capture(ProductEvent::request_revised(
                &actor.id,
                incarnation.incarnation_id(),
                &request.id,
                request.view.clone(),
            ));
    }
    state
        .publish_request_summary_refresh(
            &incarnation,
            crate::repo_events::RepoChangeReason::RequestRevised,
        )
        .await;
    if let Err(error) = crate::use_cases::request_checks::checks_view(
        &state,
        &snapshot.repository.record,
        &mutation.request,
    )
    .await
    {
        tracing::warn!(request_id = %request_id, error = %error.operator_diagnostic(), "request context accepted; check evaluation deferred");
    }
    let accepted =
        super::request_state::load(&state, &headers, &owner, &repo_name, &request_id).await?;
    Ok(Json(RequestRevisionContextResponse {
        request: super::request_state::detail_response(
            &accepted,
            super::request_state::viewer(&accepted),
        )?
        .request,
        revision_id: mutation.revision.id,
    }))
}
