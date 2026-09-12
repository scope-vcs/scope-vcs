use super::*;
use crate::api::ApiSession;
use crate::display::terminal_text;
pub(super) fn load_exact_request(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    target: RequestTargetArgs,
) -> anyhow::Result<(
    local::RequestContext,
    String,
    crate::api::RequestDetailResponse,
)> {
    let context = load_context(git_repo, api, target.remote.as_deref())?;
    let request_id = request_id_for_context(git_repo, api, &context, target.request)?;
    let detail = get_request(
        api,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )?;
    Ok((context, request_id, detail))
}

pub(super) fn api_target<'a>(
    context: &'a local::RequestContext,
    request_id: &'a str,
) -> RequestTarget<'a> {
    RequestTarget {
        owner: &context.target.owner,
        repo: &context.target.repo,
        request_id,
    }
}

pub(super) fn submit_request_command(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    target: RequestTargetArgs,
    yes: bool,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, _) = load_exact_request(git_repo, api, target)?;
    let prompt = "Submit this request to its maintainers";
    require_confirmation(prompt, yes)?;
    let response = api_submit_request(api, api_target(&context, &request_id))?;
    let human_lines = request_mutation_receipt_lines("Submitted", &response);
    Ok(RequestCommandOutcome::new(
        "request.submit",
        RequestCommandResult::Mutation(MutationResult {
            repo: context.repo,
            response,
            attachments: Vec::new(),
        }),
        human_lines,
    ))
}

pub(super) fn edit_request(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    args: args::RequestEditArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let supplied_description = args.description_file.map(text::read_markdown).transpose()?;
    let (context, request_id, before) = load_exact_request(git_repo, api, args.target)?;
    let has_attachments = !args.attachments.paths.is_empty();
    let uploaded = attachments::upload(
        api,
        api_target(&context, &request_id),
        scope_api_contract::attachments::RequestAttachmentTargetInput {
            kind: scope_api_contract::attachments::RequestAttachmentTargetKind::Description,
            discussion_id: None,
        },
        args.attachments.paths,
    )?;
    let description = if has_attachments {
        let base =
            supplied_description.unwrap_or_else(|| before.request.description_markdown.clone());
        let existing = scope_domain::requests::attachments::request_attachment_references(&base)
            .context("inspect existing request attachment references")?;
        Some(text::append_attachment_references(
            base,
            uploaded
                .attachments
                .iter()
                .zip(uploaded.references)
                .filter_map(|(attachment, reference)| {
                    (!existing.contains(&attachment.id)).then_some(reference)
                }),
        ))
    } else {
        supplied_description
    };
    let response = edit_request_identity(
        api,
        api_target(&context, &request_id),
        args.title,
        description,
        has_attachments.then(|| before.request.description_markdown.clone()),
    )?;
    let mut human_lines = request_mutation_receipt_lines("Edited request", &response);
    human_lines.extend(attachment_receipt_lines(&uploaded.attachments));
    let attachments = if args.attachments.wait {
        attachments::wait_for_processing(
            api,
            api_target(&context, &request_id),
            uploaded.attachments,
            serde_json::json!({
                "operation": "request.edit",
                "saved": true,
                "request_id": &request_id,
                "request": &response.request,
            }),
        )?
    } else {
        uploaded.attachments
    };
    attachments::complete_uploads(&uploaded.receipt_keys)?;
    Ok(RequestCommandOutcome::new(
        "request.edit",
        RequestCommandResult::Mutation(MutationResult {
            repo: context.repo,
            response,
            attachments,
        }),
        human_lines,
    ))
}

pub(super) fn attachment_receipt_lines(
    attachments: &[scope_api_contract::attachments::RequestAttachmentResponse],
) -> Vec<String> {
    attachments
        .iter()
        .map(|attachment| {
            format!(
                "Attached {} · {}",
                terminal_text(&attachment.filename),
                terminal_text(&attachment.id)
            )
        })
        .collect()
}

fn exact_handle(handle: String) -> anyhow::Result<String> {
    let handle = handle.trim().strip_prefix('@').unwrap_or(handle.trim());
    if handle.is_empty() {
        bail!("an exact Scope handle is required");
    }
    Ok(handle.to_string())
}

pub(super) fn invite_request(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    target: RequestTargetArgs,
    handle: String,
    invite: bool,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, _) = load_exact_request(git_repo, api, target)?;
    let handle = exact_handle(handle)?;
    let (command, response, human_line) = if invite {
        let response = add_request_invitee(api, api_target(&context, &request_id), handle)?;
        let human_line = invitee_added_receipt(&response);
        ("request.invite", response, human_line)
    } else {
        let response = remove_request_invitee(api, api_target(&context, &request_id), handle)?;
        let human_line = invitee_removed_receipt(&response);
        ("request.uninvite", response, human_line)
    };
    Ok(RequestCommandOutcome::new(
        command,
        RequestCommandResult::Invitee(RepoResponse {
            repo: context.repo,
            response,
        }),
        vec![human_line],
    ))
}

pub(super) fn leave_invited_request(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    target: RequestTargetArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, _) = load_exact_request(git_repo, api, target)?;
    let response = leave_request(api, api_target(&context, &request_id))?;
    let human_line = leave_receipt(&request_id, &response);
    Ok(RequestCommandOutcome::new(
        "request.leave",
        RequestCommandResult::Leave(TargetResponse {
            repo: context.repo,
            request_id,
            response,
        }),
        vec![human_line],
    ))
}

pub(super) fn merge_request_command(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    target: RequestTargetArgs,
    yes: bool,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, before) = load_exact_request(git_repo, api, target)?;
    require_confirmation(
        &format!("Merge request {} into main", before.request.name),
        yes,
    )?;
    let response = merge_request(api, api_target(&context, &request_id))?;
    let human_lines = request_mutation_receipt_lines("Merged", &response);
    Ok(RequestCommandOutcome::new(
        "request.merge",
        RequestCommandResult::Mutation(MutationResult {
            repo: context.repo,
            response,
            attachments: Vec::new(),
        }),
        human_lines,
    ))
}

pub(super) fn rate_request_command(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    target: RequestTargetArgs,
    score: u8,
    reason: String,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, _) = load_exact_request(git_repo, api, target)?;
    let response = rate_request(api, api_target(&context, &request_id), score, reason)?;
    let human_line = format!(
        "Rated @{} {}/5 — {}",
        terminal_text(&response.subject.handle),
        response.score,
        terminal_text(&response.reason)
    );
    Ok(RequestCommandOutcome::new(
        "request.rate",
        RequestCommandResult::Rating(TargetResponse {
            repo: context.repo,
            request_id,
            response,
        }),
        vec![human_line],
    ))
}

fn events_through_version(
    events: Vec<crate::api::RequestEventResponse>,
    version: u64,
) -> Vec<crate::api::RequestEventResponse> {
    events
        .into_iter()
        .filter(|event| event.position <= version)
        .collect()
}

fn full_request_activity(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    after_position: u64,
    version: u64,
) -> anyhow::Result<crate::api::RequestActivityPageResponse> {
    let mut events = Vec::new();
    let mut after = after_position;
    while after < version {
        let page = get_request_activity(
            api,
            RequestActivityParams {
                target,
                after: Some(after),
                latest: false,
                limit: Some(100),
            },
        )?;
        let page_events = events_through_version(page.events, version);
        let next = page_events
            .last()
            .map(|event| event.position)
            .unwrap_or(after);
        events.extend(page_events);
        if next == after {
            break;
        }
        after = next;
    }
    Ok(crate::api::RequestActivityPageResponse {
        events,
        through_position: version,
    })
}

pub(super) fn show_one_request(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    target: RequestTargetArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, detail) = load_exact_request(git_repo, api, target)?;
    let activity = full_request_activity(
        api,
        api_target(&context, &request_id),
        0,
        detail.request.activity_version,
    )?;
    let mut human_lines = request_detail_lines(&detail.request);
    human_lines.extend(request_activity_lines_for_response(&activity));
    Ok(RequestCommandOutcome::new(
        "request.show",
        RequestCommandResult::Detail(DetailResult {
            repo: context.repo,
            request: detail.request,
            activity: Some(activity),
        }),
        human_lines,
    ))
}

pub(super) fn list_request_status(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    args: args::RequestListArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let context = load_context(git_repo, api, args.remote.as_deref())?;
    let mut requests = load_request_list(api, &context)?;
    requests.retain(|request| {
        args.state.is_none_or(|state| request.state == state.into())
            && args
                .audience
                .is_none_or(|audience| request.audience == audience.into())
            && args.search.as_ref().is_none_or(|search| {
                let search = search.to_lowercase();
                request.name.to_lowercase().contains(&search)
                    || request.title.to_lowercase().contains(&search)
            })
    });
    requests.truncate(args.limit as usize);
    let mut human_lines = repo_access_lines(&context.repo);
    human_lines.extend(request_list_lines(&requests)?);
    Ok(RequestCommandOutcome::new(
        "request.list",
        RequestCommandResult::List(ListResult {
            repo: context.repo,
            requests,
        }),
        human_lines,
    ))
}

pub(super) fn load_request_list(
    api: ApiSession<'_>,
    context: &local::RequestContext,
) -> anyhow::Result<Vec<crate::api::RequestListItemResponse>> {
    let mut requests = Vec::new();
    let mut cursor = None;
    loop {
        let page = list_requests(
            api,
            &context.target.owner,
            &context.target.repo,
            cursor.as_deref(),
        )?;
        requests.extend(page.requests);
        let Some(next) = page.next_cursor else { break };
        cursor = Some(next);
    }
    requests.sort_by(|left, right| {
        let rank = |state| match state {
            crate::api::RequestState::Open => 0,
            crate::api::RequestState::Draft => 1,
            crate::api::RequestState::Closed => 2,
            crate::api::RequestState::Merged => 3,
        };
        let state_order = rank(left.state).cmp(&rank(right.state));
        if state_order != std::cmp::Ordering::Equal {
            return state_order;
        }
        if left.state == crate::api::RequestState::Open {
            return left
                .submitted_at_unix
                .cmp(&right.submitted_at_unix)
                .then_with(|| left.id.cmp(&right.id));
        }
        left.updated_at_unix
            .cmp(&right.updated_at_unix)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(requests)
}

pub(super) fn request_list_lines(
    requests: &[crate::api::RequestListItemResponse],
) -> anyhow::Result<Vec<String>> {
    if requests.is_empty() {
        return Ok(vec!["No visible requests.".to_string()]);
    }
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs();
    let mut lines = vec![" WAIT  STATE      REQUEST".to_string()];
    lines.extend(
        requests
            .iter()
            .map(|request| request_list_line(request, now_unix)),
    );
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn activity_events_are_bounded_to_the_response_version() {
        let events: Vec<crate::api::RequestEventResponse> = serde_json::from_value(json!([
            {
                "id": "event_2", "position": 2,
                "actor": {"id": "scope_usr_actor", "handle": "actor"},
                "kind": "Submitted",
                "payload": {"Submitted": {
                    "head_oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                }},
                "created_at_unix": 20
            },
            {
                "id": "event_3", "position": 3,
                "actor": {"id": "scope_usr_actor", "handle": "actor"},
                "kind": "Submitted",
                "payload": {"Submitted": {
                    "head_oid": "cccccccccccccccccccccccccccccccccccccccc"
                }},
                "created_at_unix": 30
            }
        ]))
        .unwrap();

        let bounded = events_through_version(events, 2);
        assert_eq!(bounded.len(), 1);
        assert_eq!(bounded[0].position, 2);
    }
}
