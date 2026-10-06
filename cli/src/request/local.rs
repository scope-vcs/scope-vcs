use crate::api::ApiSession;
use crate::display::short_oid;
use crate::{
    api::{RepoSummaryResponse, RequestSummaryResponse, get_repo, list_requests},
    error::CliError,
    git_repo::{
        GitRepo, StaleRefLease, branch_config_value, current_branch,
        fetch_scope_remote_with_bearer, git_output_in_repo, push_head_to_ref_with_bearer,
        run_git_in_repo, scope_remote_head_oid, set_branch_config_value, try_run_git_in_repo,
    },
    git_transport::ScopeRemote,
    push::DEFAULT_SCOPE_BRANCH,
};
use anyhow::{Context, bail};
use scope_api_contract::{ErrorCode, ErrorResponse, ViewId};

const REQUEST_REMOTE_KEY: &str = "scopeRequestRemote";
const REQUEST_ID_KEY: &str = "scopeRequestId";
const REQUEST_NAME_KEY: &str = "scopeRequestName";
const REQUEST_OWNER_KEY: &str = "scopeRequestOwner";
const REQUEST_REPO_KEY: &str = "scopeRequestRepo";
const REQUEST_VIEW_KEY: &str = "scopeRequestView";

pub(super) struct RequestContext {
    pub(super) target: ScopeRemote,
    pub(super) repo: RepoSummaryResponse,
}

pub(super) fn load_context(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    remote: Option<&str>,
) -> anyhow::Result<RequestContext> {
    let target = crate::context::resolve_repository(git_repo, remote)?;
    let repo = get_repo(api, &target.owner, &target.repo)?;
    Ok(RequestContext { target, repo })
}

pub(super) fn load_context_and_request_id(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    remote: Option<String>,
    request_id: Option<String>,
) -> anyhow::Result<(RequestContext, String)> {
    let context = load_context(git_repo, api, remote.as_deref())?;
    let request_id = request_id_for_context(git_repo, api, &context, request_id)?;
    Ok((context, request_id))
}

pub(super) fn refresh_main_projection(
    git_repo: &GitRepo,
    target: &ScopeRemote,
    view: &ViewId,
    session_token: &str,
) -> anyhow::Result<String> {
    fetch_main_projection(
        git_repo,
        &target.remote,
        &target.url_for_view(&view.clone().into()),
        session_token,
    )
}

fn fetch_main_projection(
    git_repo: &GitRepo,
    remote: &str,
    fetch_url: &str,
    session_token: &str,
) -> anyhow::Result<String> {
    fetch_scope_remote_with_bearer(
        git_repo,
        fetch_url,
        remote,
        DEFAULT_SCOPE_BRANCH,
        session_token,
    )?;
    scope_remote_head_oid(git_repo, remote, DEFAULT_SCOPE_BRANCH)?
        .context("Scope main projection did not produce a local remote ref")
}

const STALE_REQUEST_PUSH_ERROR: &str = "Someone else updated this request. Fetch it and try again.";

pub(super) fn push_request_head(
    target: &ScopeRemote,
    session_token: &str,
    request_head_oid: &str,
    expected_head_oid: &str,
    request_id: &str,
    request_name: &str,
) -> anyhow::Result<()> {
    let request_ref = format!("refs/heads/{request_name}");
    push_head_to_ref_with_bearer(
        &target.full_view_url(),
        request_head_oid,
        &request_ref,
        expected_head_oid,
        session_token,
    )
    .map_err(|error| {
        if error.is::<StaleRefLease>() {
            CliError::new(ErrorResponse::new(
                ErrorCode::Conflict,
                STALE_REQUEST_PUSH_ERROR,
            ))
            .into()
        } else {
            error.context(format!("push request branch for {request_id}"))
        }
    })
}

pub(super) fn last_seen_request_head(
    git_repo: &GitRepo,
    target: &ScopeRemote,
    request: &RequestSummaryResponse,
    branch: &str,
    request_head_oid: &str,
) -> anyhow::Result<String> {
    let lease = scope_remote_head_oid(git_repo, &target.remote, &request.name)?
        .unwrap_or_else(|| request.head_oid.as_str().to_string());
    if git_is_ancestor(git_repo, &lease, request_head_oid)?
        || branch_reflog_includes(git_repo, branch, &lease)?
    {
        return Ok(lease);
    }
    Err(CliError::new(ErrorResponse::new(
        ErrorCode::Conflict,
        STALE_REQUEST_PUSH_ERROR,
    ))
    .into())
}

fn branch_reflog_includes(git_repo: &GitRepo, branch: &str, oid: &str) -> anyhow::Result<bool> {
    let entries = git_output_in_repo(
        git_repo,
        &[
            "log",
            "--walk-reflogs",
            "--format=%H",
            &format!("refs/heads/{branch}"),
        ],
    )?;
    if !entries.status.success() {
        return Ok(false);
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in String::from_utf8_lossy(&entries.stdout).lines() {
        if seen.insert(entry) && git_is_ancestor(git_repo, oid, entry)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn git_is_ancestor(git_repo: &GitRepo, ancestor: &str, descendant: &str) -> anyhow::Result<bool> {
    Ok(git_output_in_repo(
        git_repo,
        &["merge-base", "--is-ancestor", ancestor, descendant],
    )?
    .status
    .success())
}

pub(super) fn request_id_for_context(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    context: &RequestContext,
    request_id: Option<String>,
) -> anyhow::Result<String> {
    maybe_request_id_for_context(git_repo, api, context, request_id)?.ok_or_else(|| {
        crate::error::CliError::usage(
            "select a visible request with --request <name-or-id>, or check out its branch",
        )
        .into()
    })
}

pub(super) fn maybe_request_id_for_context(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    context: &RequestContext,
    request_id: Option<String>,
) -> anyhow::Result<Option<String>> {
    let explicit = normalized_optional_arg(request_id);
    if let Some(request_id) = explicit
        .as_deref()
        .filter(|value| value.starts_with("req_"))
    {
        return Ok(Some(request_id.to_string()));
    }
    let request_name = if let Some(request_name) = explicit {
        request_name
    } else if let Some(git_repo) = git_repo {
        let branch = match current_branch(git_repo) {
            Ok(branch) => branch,
            Err(_) => return Ok(None),
        };
        if let Some(attachment) = request_attachment(git_repo, &branch)? {
            validate_stored_request_target(&attachment, context)?;
            return Ok(Some(attachment.id));
        }
        let tracking_remote = branch_config_value(git_repo, &branch, "remote")?;
        let merge_ref = branch_config_value(git_repo, &branch, "merge")?;
        inferred_request_name(
            branch,
            &context.target.remote,
            tracking_remote.as_deref(),
            merge_ref.as_deref(),
        )
    } else {
        return Ok(None);
    };
    let mut cursor = None;
    loop {
        let page = list_requests(
            api,
            &context.target.owner,
            &context.target.repo,
            cursor.as_deref(),
        )?;
        if let Some(request) = page
            .requests
            .into_iter()
            .find(|request| request.name == request_name)
        {
            return Ok(Some(request.id));
        }
        let Some(next_cursor) = page.next_cursor else {
            return Ok(None);
        };
        cursor = Some(next_cursor);
    }
}

fn inferred_request_name(
    branch: String,
    selected_remote: &str,
    tracking_remote: Option<&str>,
    merge_ref: Option<&str>,
) -> String {
    if tracking_remote == Some(selected_remote)
        && let Some(request_name) =
            merge_ref.and_then(|request_ref| request_ref.strip_prefix("refs/heads/"))
    {
        return request_name.to_string();
    }
    branch
}

struct RequestAttachment {
    id: String,
    owner: Option<String>,
    repo: Option<String>,
    remote: Option<String>,
    name: Option<String>,
}

fn request_attachment(
    git_repo: &GitRepo,
    branch: &str,
) -> anyhow::Result<Option<RequestAttachment>> {
    let Some(id) = branch_config_value(git_repo, branch, REQUEST_ID_KEY)? else {
        return Ok(None);
    };
    Ok(Some(RequestAttachment {
        id,
        owner: branch_config_value(git_repo, branch, REQUEST_OWNER_KEY)?,
        repo: branch_config_value(git_repo, branch, REQUEST_REPO_KEY)?,
        remote: branch_config_value(git_repo, branch, REQUEST_REMOTE_KEY)?,
        name: branch_config_value(git_repo, branch, REQUEST_NAME_KEY)?,
    }))
}

pub(crate) enum RequestComparison {
    Main,
    Request(String),
    Unavailable(String),
}

pub(crate) fn resolve_request_comparison_ref(
    git_repo: &GitRepo,
    branch: &str,
    target: &ScopeRemote,
    resolved_request: Option<&RequestSummaryResponse>,
) -> RequestComparison {
    let Some(attachment) = request_attachment(git_repo, branch).ok().flatten() else {
        return RequestComparison::Main;
    };
    if attachment.id.is_empty()
        || attachment.owner.as_deref() != Some(target.owner.as_str())
        || attachment.repo.as_deref() != Some(target.repo.as_str())
        || attachment.remote.as_deref() != Some(target.remote.as_str())
    {
        return RequestComparison::Main;
    }
    let name = resolved_request
        .filter(|request| request.id == attachment.id)
        .map(|request| request.name.as_str())
        .or(attachment.name.as_deref());
    match name {
        Some(name) if !name.is_empty() => {
            RequestComparison::Request(request_remote_ref(&target.remote, name))
        }
        _ => RequestComparison::Unavailable(attachment.id),
    }
}

fn validate_stored_request_target(
    attachment: &RequestAttachment,
    context: &RequestContext,
) -> anyhow::Result<()> {
    match (attachment.owner.as_deref(), attachment.repo.as_deref()) {
        (Some(owner), Some(repo))
            if owner == context.target.owner && repo == context.target.repo =>
        {
            Ok(())
        }
        (Some(owner), Some(repo)) => bail!(
            "current branch belongs to Scope repository {owner}/{repo}, but remote {} targets {}/{}; pass the correct --remote",
            context.target.remote,
            context.target.owner,
            context.target.repo
        ),
        _ => bail!(
            "current branch request metadata is incomplete; pass --request <name-or-id> explicitly"
        ),
    }
}

pub(super) fn store_request_metadata(
    git_repo: &GitRepo,
    branch: &str,
    context: &RequestContext,
    request: &RequestSummaryResponse,
) -> anyhow::Result<()> {
    set_branch_config_value(git_repo, branch, REQUEST_OWNER_KEY, &context.target.owner)?;
    set_branch_config_value(git_repo, branch, REQUEST_REPO_KEY, &context.target.repo)?;
    set_branch_config_value(git_repo, branch, REQUEST_REMOTE_KEY, &context.target.remote)?;
    set_branch_config_value(git_repo, branch, REQUEST_ID_KEY, &request.id)?;
    set_branch_config_value(git_repo, branch, REQUEST_NAME_KEY, &request.name)?;
    set_branch_config_value(git_repo, branch, REQUEST_VIEW_KEY, request.view.as_str())
}

pub(super) fn adoptable_current_branch(
    git_repo: &GitRepo,
    base_oid: &str,
) -> anyhow::Result<String> {
    let branch = current_branch(git_repo)?;
    if branch == DEFAULT_SCOPE_BRANCH {
        return Err(crate::error::CliError::usage(
            "main cannot become a request branch; switch to the branch that holds the work",
        )
        .into());
    }
    if let Some(request_id) = branch_config_value(git_repo, &branch, REQUEST_ID_KEY)? {
        return Err(crate::error::CliError::usage(format!(
            "branch '{branch}' already belongs to request {request_id}"
        ))
        .into());
    }
    if !try_run_git_in_repo(git_repo, &["merge-base", "--is-ancestor", base_oid, "HEAD"])? {
        return Err(crate::error::CliError::usage(format!(
            "branch '{branch}' does not contain Scope main at {}; merge or rebase onto it, then retry",
            short_oid(base_oid)
        ))
        .into());
    }
    Ok(branch)
}

pub(super) fn track_request_branch_ref(
    git_repo: &GitRepo,
    branch: &str,
    target: &ScopeRemote,
    request_name: &str,
    request_head_oid: &str,
) -> anyhow::Result<()> {
    update_request_remote_ref(git_repo, target, request_name, request_head_oid)?;
    if branch_config_value(git_repo, branch, "remote")?
        .is_some_and(|remote| remote != target.remote)
    {
        return Ok(());
    }
    set_branch_config_value(git_repo, branch, "remote", &target.remote)?;
    set_branch_config_value(
        git_repo,
        branch,
        "merge",
        &format!("refs/heads/{request_name}"),
    )
}

pub(super) fn update_request_remote_ref(
    git_repo: &GitRepo,
    target: &ScopeRemote,
    request_name: &str,
    request_head_oid: &str,
) -> anyhow::Result<()> {
    let remote_ref = request_remote_ref(&target.remote, request_name);
    run_git_in_repo(git_repo, &["update-ref", &remote_ref, request_head_oid])
}

pub(super) fn remote_main_ref(remote: &str) -> String {
    format!("refs/remotes/{remote}/{DEFAULT_SCOPE_BRANCH}")
}

pub(super) fn request_remote_ref(remote: &str, request_name: &str) -> String {
    format!("refs/remotes/{remote}/{request_name}")
}

fn normalized_optional_arg(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
pub(super) fn require_git_remote(context: &RequestContext) -> anyhow::Result<()> {
    if context.target.remote.is_empty() {
        return Err(crate::error::CliError::usage(
            "this operation requires a configured Scope Git remote; pass --remote <name>",
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{fetch_main_projection, inferred_request_name};
    use crate::{git_repo::GitRepo, test_support::TempDir};
    use std::fs;

    #[test]
    fn merge_ref_only_names_a_request_for_the_selected_scope_remote() {
        assert_eq!(
            inferred_request_name(
                "scope-fix".to_string(),
                "scope",
                Some("scope"),
                Some("refs/heads/request-fix"),
            ),
            "request-fix"
        );
        assert_eq!(
            inferred_request_name(
                "scope-fix".to_string(),
                "scope",
                Some("origin"),
                Some("refs/heads/other-fix"),
            ),
            "scope-fix"
        );
    }

    #[test]
    fn main_projection_refresh_follows_alternating_request_views() {
        let (public, public_oid) = repository_with_commit("public-main", "public.txt");
        let (private, private_oid) = repository_with_commit("private-main", "private.txt");
        let checkout = TempDir::git_repo("alternating-main", "main");
        let repo = GitRepo {
            root: checkout.path().to_path_buf(),
        };
        let (public_url, private_url) = (file_url(public.path()), file_url(private.path()));

        for (url, oid) in [
            (&public_url, &public_oid),
            (&private_url, &private_oid),
            (&public_url, &public_oid),
        ] {
            assert_eq!(
                &fetch_main_projection(&repo, "scope", url, "unused").unwrap(),
                oid
            );
        }
    }

    fn repository_with_commit(label: &str, file: &str) -> (TempDir, String) {
        let dir = TempDir::git_repo(label, "main");
        dir.run_git(["config", "user.email", "scope@example.test"]);
        dir.run_git(["config", "user.name", "Scope Test"]);
        fs::write(dir.path().join(file), format!("{label}\n")).unwrap();
        dir.run_git(["add", file]);
        dir.run_git(["commit", "-m", label]);
        let oid = String::from_utf8(dir.run_git(["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        (dir, oid)
    }

    fn file_url(path: &std::path::Path) -> String {
        format!("file://{}", path.display())
    }
}
