use super::*;
use scope_api_contract::{
    RequestBase, RequestRevisionContextRequest, RequestState, RequestSummaryResponse,
};

pub(super) struct ParentPin {
    pub(super) base: RequestBase,
    pub(super) base_version: u64,
}

pub(super) fn current_parent(
    git_repo: &GitRepo,
    api: ApiSession<'_>,
    context: &local::RequestContext,
    selector: String,
    view: Option<&ViewId>,
    revision: Option<&str>,
) -> anyhow::Result<(RequestSummaryResponse, ParentPin)> {
    let id = request_id_for_context(Some(git_repo), api, context, Some(selector))?;
    let parent = get_request(api, &context.target.owner, &context.target.repo, &id)?.request;
    if parent.state != RequestState::Open {
        return Err(crate::error::CliError::usage("the parent request must be Open").into());
    }
    if scope_domain::views::ViewId::from(parent.view.clone()) != *context.views.full()
        || view.is_some_and(|view| view != &parent.view)
    {
        return Err(crate::error::CliError::usage(
            "stacked requests require the same private full view",
        )
        .into());
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
    let pin = ParentPin {
        base: RequestBase::Request {
            request_id: id,
            revision_id,
            head_oid: parent.head_oid.clone(),
        },
        base_version: parent.base_version,
    };
    Ok((parent, pin))
}

pub(super) fn require_contains_pin(
    git_repo: &GitRepo,
    base: &RequestBase,
    head: &str,
) -> anyhow::Result<()> {
    if let RequestBase::Request { head_oid, .. } = base
        && !try_run_git_in_repo(
            git_repo,
            &["merge-base", "--is-ancestor", head_oid.as_str(), head],
        )?
    {
        return Err(crate::error::CliError::usage(
            "the commit does not contain the pinned parent head; manually restack before pushing",
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
    pin: &ParentPin,
    head: &str,
) -> anyhow::Result<()> {
    let key = format!("scope.request-{}.revisionContext", request.id);
    let saved = crate::git_repo::git_output_in_repo(git_repo, &["config", "--get", &key])?;
    let body = if saved.status.success() {
        let body: RequestRevisionContextRequest = serde_json::from_slice(&saved.stdout)?;
        if body.expected_head_oid.as_str() != head || body.base != pin.base {
            return Err(crate::error::CliError::new(ErrorResponse::new(ErrorCode::Conflict, "a previous revision-context publication needs recovery; retry with the same head and parent pin")).into());
        }
        body
    } else {
        let revision_id =
            crate::api::request_revisions(api, api_target(context, &request.id), None, None, None)?
                .review_revision_id
                .context("request has no saved revision")?;
        let body = RequestRevisionContextRequest {
            operation_key: new_client_mutation_id("revision-context")?,
            base: pin.base.clone(),
            expected_head_oid: head.try_into()?,
            expected_revision_id: revision_id,
            expected_base_version: request.base_version,
            expected_parent_base_version: pin.base_version,
        };
        run_git_in_repo(git_repo, &["config", &key, &serde_json::to_string(&body)?])?;
        body
    };
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
                            && revision.base_version == body.expected_base_version.saturating_add(1)
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
                        "Publication may already be committed. Retry with the same local head and --base-revision; the saved operation key will recover its accepted revision.",
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
            && revision.base_version == body.expected_base_version.saturating_add(1)
    }) {
        return Err(crate::error::CliError::new(ErrorResponse::new(
            ErrorCode::Conflict,
            "the accepted revision does not confirm the intended head and parent pin",
        ))
        .into());
    }
    run_git_in_repo(git_repo, &["config", "--unset", &key])
}

pub(super) fn publication_headers(
    request: &RequestSummaryResponse,
    pin: &ParentPin,
) -> Vec<String> {
    let RequestBase::Request {
        request_id,
        revision_id,
        head_oid,
    } = &pin.base
    else {
        return Vec::new();
    };
    vec![
        format!("Scope-Request-Base: request:{request_id}@{revision_id}"),
        format!("Scope-Request-Base-Version: {}", request.base_version),
        format!("Scope-Request-Parent-Base-Version: {}", pin.base_version),
        format!("Scope-Request-Parent-Head: {head_oid}"),
    ]
}

pub(super) fn accepted_pin(
    api: ApiSession<'_>,
    context: &local::RequestContext,
    request: &RequestSummaryResponse,
    head: &str,
    parent_revision: &str,
    expected_publication: Option<(&RequestBase, u64)>,
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
    if revision.new_head_oid.as_deref() != Some(head)
        || !matches!(&revision.base, RequestBase::Request { revision_id, .. } if revision_id == parent_revision)
    {
        return Ok(false);
    }
    if let Some((base, version)) = expected_publication {
        return Ok(revision.base == *base && revision.base_version == version);
    }
    if request.head_oid.as_str() != head {
        return Ok(false);
    }
    Ok(match &request.base {
        RequestBase::Request { .. } => {
            request.base == revision.base && request.base_version == revision.base_version
        }
        RequestBase::Main { review_base_oid } => {
            matches!(&revision.base, RequestBase::Request { head_oid, .. } if head_oid == review_base_oid)
                && revision.base_version.checked_add(1) == Some(request.base_version)
        }
    })
}

pub(super) fn retry_pending_context(
    git_repo: &GitRepo,
    api: ApiSession<'_>,
    context: &local::RequestContext,
    request: &RequestSummaryResponse,
    head: &str,
    revision: &str,
) -> anyhow::Result<bool> {
    let key = format!("scope.request-{}.revisionContext", request.id);
    let saved = crate::git_repo::git_output_in_repo(git_repo, &["config", "--get", &key])?;
    if !saved.status.success() {
        return Ok(false);
    }
    let body: RequestRevisionContextRequest = serde_json::from_slice(&saved.stdout)?;
    if !matches!(&body.base, RequestBase::Request { revision_id, .. } if revision_id == revision) {
        return Err(crate::error::CliError::new(ErrorResponse::new(
            ErrorCode::Conflict,
            "retry the pending revision-context publication with its original parent revision",
        ))
        .into());
    }
    let pin = ParentPin {
        base: body.base,
        base_version: body.expected_parent_base_version,
    };
    publish_context(git_repo, api, context, request, &pin, head)?;
    Ok(true)
}
