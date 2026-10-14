use super::*;
use scope_api_contract::{
    RequestBase, RequestRevisionContextRequest, RequestState, RequestSummaryResponse,
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SelectedBase {
    pub(super) base: RequestBase,
    pub(super) parent_base_version: Option<u64>,
}

pub(super) fn current_parent(
    git_repo: &GitRepo,
    api: ApiSession<'_>,
    context: &local::RequestContext,
    selector: String,
    view: Option<&ViewId>,
    revision: Option<&str>,
) -> anyhow::Result<(RequestSummaryResponse, SelectedBase)> {
    let id = request_id_for_context(Some(git_repo), api, context, Some(selector))?;
    let parent = get_request(api, &context.target.owner, &context.target.repo, &id)?.request;
    if parent.state != RequestState::Open {
        return Err(crate::error::CliError::usage("the parent request must be Open").into());
    }
    if view.is_some_and(|view| view != &parent.view) {
        return Err(crate::error::CliError::usage("stacked requests require the same view").into());
    }
    let revision_id =
        crate::api::request_revisions(api, api_target(context, &id), None, None, None)?
            .review_revision_id
            .context("the parent has no saved revision")?;
    if revision.is_some_and(|selected| selected != revision_id) {
        return Err(crate::error::CliError::new(ErrorResponse::new(
            ErrorCode::Conflict,
            "--base-revision must select the parent's current saved revision",
        ))
        .into());
    }
    let fetched = crate::git_repo::fetch_scope_parent_with_bearer(
        git_repo,
        &context.target.url_for_view(&parent.view.clone().into()),
        &parent.name,
        &id,
        api.token,
    )?;
    if fetched != parent.head_oid.as_str() {
        return Err(crate::error::CliError::new(ErrorResponse::new(
            ErrorCode::Conflict,
            "the parent moved during fetch; retry with its current revision",
        ))
        .into());
    }
    let pin = SelectedBase {
        base: RequestBase::Request {
            request_id: id,
            revision_id,
            head_oid: parent.head_oid.clone(),
        },
        parent_base_version: Some(parent.base_version),
    };
    Ok((parent, pin))
}

pub(super) fn require_contains_pin(
    git_repo: &GitRepo,
    base: &RequestBase,
    head: &str,
) -> anyhow::Result<()> {
    let head_oid = base_head(base);
    if !try_run_git_in_repo(git_repo, &["merge-base", "--is-ancestor", head_oid, head])? {
        return Err(crate::error::CliError::usage(
            "the commit does not contain the selected base; restack before pushing",
        )
        .into());
    }
    Ok(())
}

pub(super) fn publish_context(
    git_repo: &GitRepo,
    api: ApiSession<'_>,
    context: &local::RequestContext,
    request: &RequestSummaryResponse,
    selection: &SelectedBase,
    head: &str,
) -> anyhow::Result<()> {
    let key = format!("scope.request-{}.revisionContext", request.id);
    let saved = crate::git_repo::git_output_in_repo(git_repo, &["config", "--get", &key])?;
    let body = if saved.status.success() {
        let body: RequestRevisionContextRequest = serde_json::from_slice(&saved.stdout)?;
        if body.expected_head_oid.as_str() != head || body.base != selection.base {
            return Err(crate::error::CliError::new(ErrorResponse::new(ErrorCode::Conflict, "a previous revision-context publication needs recovery; retry with the same head and base selection")).into());
        }
        body
    } else {
        let revision_id =
            crate::api::request_revisions(api, api_target(context, &request.id), None, None, None)?
                .review_revision_id
                .ok_or_else(|| crate::error::CliError::usage("this request has no saved revision; commit its own changes, then run scope request push with the same base selection"))?;
        let body = RequestRevisionContextRequest {
            operation_key: new_client_mutation_id("revision-context")?,
            base: selection.base.clone(),
            expected_head_oid: head.try_into()?,
            expected_revision_id: revision_id,
            expected_base_version: request.base_version,
            expected_parent_base_version: selection.parent_base_version,
        };
        run_git_in_repo(git_repo, &["config", &key, &serde_json::to_string(&body)?])?;
        body
    };
    let expected_base_version = body
        .expected_base_version
        .checked_add(1)
        .context("request base version cannot advance")?;
    let receipt = match crate::api::request_revision_context(
        api,
        api_target(context, &request.id),
        &body,
    ) {
        Ok(receipt) => receipt,
        Err(error) => {
            let mut rejected = false;
            if error
                .downcast_ref::<crate::error::CliError>()
                .is_some_and(|error| {
                    matches!(
                        error.response().code,
                        ErrorCode::BadRequest
                            | ErrorCode::Conflict
                            | ErrorCode::Forbidden
                            | ErrorCode::NotFound
                            | ErrorCode::Unauthorized
                            | ErrorCode::ProtectedPath
                    )
                })
                && let Ok(history) = crate::api::request_revisions(
                    api,
                    api_target(context, &request.id),
                    None,
                    None,
                    None,
                )
            {
                let boundary = history
                    .revisions
                    .iter()
                    .find(|revision| revision.id == body.expected_revision_id)
                    .map(|revision| revision.position);
                let accepted = history
                    .revisions
                    .iter()
                    .filter(|revision| boundary.is_none_or(|position| revision.position > position))
                    .any(|revision| {
                        revision.new_head_oid.as_deref() == Some(head)
                            && revision.base == body.base
                            && revision.base_version == expected_base_version
                    });
                if (!history.has_earlier_revisions || boundary.is_some()) && !accepted {
                    run_git_in_repo(git_repo, &["config", "--unset", &key]).context(
                        "clear rejected revision context after confirming immutable history",
                    )?;
                    rejected = true;
                }
            }
            if !rejected {
                return Err(crate::error::CliError::new(crate::error::response(&error).with_instruction(
                        "Publication may already be committed. Retry with the same local head and base selection; the saved operation key will recover its accepted revision.",
                    )).into());
            }
            return Err(error);
        }
    };
    let accepted = crate::api::request_revisions(
        api,
        api_target(context, &request.id),
        Some(&receipt.revision_id),
        None,
        None,
    )?;
    if !accepted.revisions.iter().any(|revision| {
        revision.id == receipt.revision_id
            && revision.new_head_oid.as_deref() == Some(head)
            && revision.base == body.base
            && revision.base_version == expected_base_version
    }) {
        return Err(crate::error::CliError::new(ErrorResponse::new(
            ErrorCode::Conflict,
            "the accepted revision does not confirm the intended head and base",
        ))
        .into());
    }
    run_git_in_repo(git_repo, &["config", "--unset", &key])
}

pub(super) fn publication_headers(
    request: &RequestSummaryResponse,
    selection: &SelectedBase,
) -> Vec<String> {
    let mut headers = vec![format!(
        "Scope-Request-Base-Version: {}",
        request.base_version
    )];
    match &selection.base {
        RequestBase::Main { review_base_oid } => {
            headers.push(format!("Scope-Request-Base: main:{review_base_oid}"))
        }
        RequestBase::Request {
            request_id,
            revision_id,
            head_oid,
        } => {
            headers.extend([
                format!("Scope-Request-Base: request:{request_id}@{revision_id}"),
                format!(
                    "Scope-Request-Parent-Base-Version: {}",
                    selection
                        .parent_base_version
                        .expect("request selection has parent version")
                ),
                format!("Scope-Request-Parent-Head: {head_oid}"),
            ]);
        }
    }
    headers
}

pub(super) fn accepted_base(
    api: ApiSession<'_>,
    context: &local::RequestContext,
    request: &RequestSummaryResponse,
    head: &str,
    base: &RequestBase,
    expected_version: Option<u64>,
) -> anyhow::Result<bool> {
    let revisions =
        crate::api::request_revisions(api, api_target(context, &request.id), None, None, None)?;
    let Some(revision) = revisions.review_revision_id.as_ref().and_then(|id| {
        revisions
            .revisions
            .iter()
            .find(|revision| &revision.id == id)
    }) else {
        return Ok(false);
    };
    if revision.new_head_oid.as_deref() != Some(head) || revision.base != *base {
        return Ok(false);
    }
    if let Some(version) = expected_version {
        return Ok(revision.base_version == version);
    }
    if request.head_oid.as_str() != head {
        return Ok(false);
    }
    Ok(
        (request.base == revision.base && request.base_version == revision.base_version)
            || matches!((&request.base, &revision.base), (RequestBase::Main { review_base_oid }, RequestBase::Request { head_oid, .. }) if review_base_oid == head_oid && revision.base_version.checked_add(1) == Some(request.base_version)),
    )
}

pub(super) fn pending_context(
    git_repo: &GitRepo,
    request: &RequestSummaryResponse,
) -> anyhow::Result<Option<SelectedBase>> {
    let key = format!("scope.request-{}.revisionContext", request.id);
    let saved = crate::git_repo::git_output_in_repo(git_repo, &["config", "--get", &key])?;
    if !saved.status.success() {
        return Ok(None);
    }
    let body: RequestRevisionContextRequest = serde_json::from_slice(&saved.stdout)?;
    Ok(Some(SelectedBase {
        base: body.base,
        parent_base_version: body.expected_parent_base_version,
    }))
}

pub(super) fn retry_pending_context(
    git_repo: &GitRepo,
    api: ApiSession<'_>,
    context: &local::RequestContext,
    request: &RequestSummaryResponse,
    head: &str,
    selection: &SelectedBase,
) -> anyhow::Result<bool> {
    let Some(pending) = pending_context(git_repo, request)? else {
        return Ok(false);
    };
    if pending.base != selection.base {
        return Err(crate::error::CliError::new(ErrorResponse::new(
            ErrorCode::Conflict,
            "retry the pending revision-context publication with its original base",
        ))
        .into());
    }
    publish_context(git_repo, api, context, request, &pending, head)?;
    let current = get_request(
        api,
        &context.target.owner,
        &context.target.repo,
        &request.id,
    )?;
    if !accepted_base(api, context, &current.request, head, &selection.base, None)? {
        return Err(crate::error::CliError::with_recovery(
            ErrorResponse::new(ErrorCode::Conflict, "the recovered revision-context publication is no longer the current request revision")
                .with_instruction("Inspect the current request and revision history before publishing again."),
            serde_json::json!({ "request": current.request }),
        ).into());
    }
    Ok(true)
}

pub(super) fn base_head(base: &RequestBase) -> &str {
    match base {
        RequestBase::Main { review_base_oid } => review_base_oid.as_str(),
        RequestBase::Request { head_oid, .. } => head_oid.as_str(),
    }
}

pub(super) fn publication_base_version(
    request: &RequestSummaryResponse,
    base: &RequestBase,
) -> anyhow::Result<u64> {
    if request.base == *base {
        Ok(request.base_version)
    } else {
        request
            .base_version
            .checked_add(1)
            .context("request base version cannot advance")
    }
}

pub(super) fn select_base(
    git_repo: &GitRepo,
    api: ApiSession<'_>,
    context: &local::RequestContext,
    request: &RequestSummaryResponse,
    selector: &str,
    revision: Option<&str>,
) -> anyhow::Result<SelectedBase> {
    if selector == "main" {
        let head = refresh_main_projection(git_repo, &context.target, &request.view, api.token)?;
        return Ok(SelectedBase {
            base: RequestBase::Main {
                review_base_oid: head.try_into()?,
            },
            parent_base_version: None,
        });
    }
    let (_, selection) = current_parent(
        git_repo,
        api,
        context,
        selector.to_string(),
        Some(&request.view),
        revision,
    )?;
    Ok(selection)
}

pub(super) fn selector_matches(
    base: &RequestBase,
    selector: Option<&str>,
    revision: Option<&str>,
    parent_name: Option<&str>,
) -> bool {
    match base {
        RequestBase::Main { .. } => selector == Some("main") && revision.is_none(),
        RequestBase::Request {
            request_id,
            revision_id,
            ..
        } => {
            revision.is_none_or(|selected| selected == revision_id)
                && selector
                    .is_none_or(|selected| selected == request_id || Some(selected) == parent_name)
        }
    }
}
