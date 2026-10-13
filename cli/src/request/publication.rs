use super::*;
use scope_api_contract::RequestBase;

pub(super) enum ExplicitBase<'a> {
    Base(&'a str),
    Revision(&'a str),
}

pub(super) fn selected_for_push(
    repo: &GitRepo,
    api: ApiSession<'_>,
    context: &local::RequestContext,
    request: &crate::api::RequestSummaryResponse,
    head: &str,
    explicit: Option<ExplicitBase<'_>>,
    pending: Option<&restack::state::PendingRestack>,
) -> anyhow::Result<Option<stack::SelectedBase>> {
    let (selector, revision) = match explicit {
        Some(ExplicitBase::Base(value)) => (Some(value), None),
        Some(ExplicitBase::Revision(value)) => (None, Some(value)),
        None => (None, None),
    };
    let saved = stack::pending_context(repo, request)?;
    let retained = pending.map(|pending| pending.selected.clone()).or(saved);
    if let Some(selected) = retained {
        if selector.is_some() || revision.is_some() {
            let parent_name = match &selected.base {
                RequestBase::Request { request_id, .. } => Some(
                    get_request(api, &context.target.owner, &context.target.repo, request_id)?
                        .request
                        .name,
                ),
                RequestBase::Main { .. } => None,
            };
            if !stack::selector_matches(&selected.base, selector, revision, parent_name.as_deref())
            {
                return Err(crate::error::CliError::new(ErrorResponse::new(
                    ErrorCode::Conflict,
                    "the explicit base disagrees with the pending publication selection",
                ))
                .into());
            }
        }
        return Ok(Some(selected));
    }
    if let Some(revision) = revision {
        let history =
            crate::api::request_revisions(api, api_target(context, &request.id), None, None, None)?;
        if let Some(accepted) = history
            .review_revision_id
            .as_ref()
            .and_then(|id| history.revisions.iter().find(|row| &row.id == id))
            && accepted.new_head_oid.as_deref() == Some(head)
            && matches!(&accepted.base, RequestBase::Request { revision_id, .. } if revision_id == revision)
            && stack::accepted_base(api, context, request, head, &accepted.base, None)?
        {
            return Ok(Some(stack::SelectedBase {
                base: accepted.base.clone(),
                parent_base_version: None,
            }));
        }
        let RequestBase::Request { request_id, .. } = &request.base else {
            return Err(crate::error::CliError::usage(
                "--base-revision requires a request with a parent",
            )
            .into());
        };
        return stack::select_base(repo, api, context, request, request_id, Some(revision))
            .map(Some);
    }
    selector
        .map(|selector| stack::select_base(repo, api, context, request, selector, None))
        .transpose()
}

pub(super) fn publish_selected(
    repo: &GitRepo,
    api: ApiSession<'_>,
    context: &local::RequestContext,
    request: &crate::api::RequestSummaryResponse,
    branch_head: (&str, &str),
    selected: Option<&stack::SelectedBase>,
    pending: Option<&restack::state::PendingRestack>,
) -> anyhow::Result<()> {
    let (branch, head) = branch_head;
    if let Some(selected) = selected {
        if stack::retry_pending_context(repo, api, context, request, head, selected)?
            || stack::accepted_base(api, context, request, head, &selected.base, None)?
        {
            return Ok(());
        }
        if let Some(pending) = pending {
            let history = crate::api::request_revisions(
                api,
                api_target(context, &request.id),
                None,
                None,
                None,
            )?;
            if request.head_oid.as_str() != pending.expected_head
                || request.base_version != pending.expected_base_version
                || history.review_revision_id.as_deref() != pending.expected_revision.as_deref()
            {
                return Err(restack::recovery(
                    repo,
                    pending,
                    "verify_publication_context",
                    "the request changed after local restacking; inspect the current request before publishing",
                ));
            }
        }
        stack::require_contains_pin(repo, &selected.base, head)?;
        restack::require_preserved_main(repo, request, &selected.base)?;
        if head == request.head_oid.as_str() {
            stack::publish_context(repo, api, context, request, selected, head)?;
            let current = get_request(
                api,
                &context.target.owner,
                &context.target.repo,
                &request.id,
            )?
            .request;
            if !stack::accepted_base(api, context, &current, head, &selected.base, None)? {
                return Err(crate::error::CliError::with_recovery(
                    ErrorResponse::new(
                        ErrorCode::Conflict,
                        "the published context is no longer the current request revision",
                    ),
                    serde_json::json!({"request":current}),
                )
                .into());
            }
            return Ok(());
        }
        let expected_head = last_seen_request_head(repo, &context.target, request, branch, head)?;
        let result = crate::git_repo::push_head_to_ref_with_bearer(
            &context.target.url_for_view(&request.view.clone().into()),
            head,
            &format!("refs/heads/{}", request.name),
            &expected_head,
            api.token,
            &stack::publication_headers(request, selected),
        )
        .map_err(|error| local::request_push_error(error, &request.id));
        let expected_version = stack::publication_base_version(request, &selected.base)?;
        let current = get_request(
            api,
            &context.target.owner,
            &context.target.repo,
            &request.id,
        )?
        .request;
        if !stack::accepted_base(
            api,
            context,
            &current,
            head,
            &selected.base,
            Some(expected_version),
        )? || current.head_oid.as_str() != head
        {
            let transport_error = match result {
                Err(error) if crate::error::response(&error).code != ErrorCode::Internal => {
                    return Err(error);
                }
                Err(error) => Some(crate::error::response(&error).message),
                Ok(()) => None,
            };
            return Err(crate::error::CliError::with_recovery(
                ErrorResponse::new(
                    ErrorCode::Conflict,
                    "revision history cannot confirm the intended head and base",
                ),
                serde_json::json!({"request":current,"selected_base":selected.base,"intended_head_oid":head,"transport_error":transport_error}),
            )
            .into());
        }
        Ok(())
    } else {
        stack::require_contains_pin(repo, &request.base, head)?;
        let expected_head = last_seen_request_head(repo, &context.target, request, branch, head)?;
        push_request_head(
            &context.target,
            &request.view,
            api.token,
            head,
            &expected_head,
            &request.id,
            &request.name,
        )
    }
}
