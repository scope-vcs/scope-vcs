use crate::{
    auth::{
        scope::{optional_scope_user, require_scope_user},
        tokens::{generate_repository_invite_token, random_token, token_hash},
    },
    error::ApiError,
    http::{origins::public_app_origin, responses::*},
    persistence::unix_now,
    repo_access::find_read_access,
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::repository_collaboration::{
        accept_repository_invite as accept_invite, map_committed_mutation,
        publish_committed_mutation,
    },
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_domain::{
    account::UserAccount,
    repo_collaboration::{AcceptRepositoryInviteOutcome, repository_invite_landing},
    repository::access::RepositoryAccess,
    requests::{Request, RequestViewer, request_policy},
};

pub(crate) async fn list_repository_collaboration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepositoryCollaborationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let collaboration = state
        .metadata
        .repositories()
        .repository_collaboration(&owner, &repo_name, &user.id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    Ok(Json(repository_collaboration_response(
        &collaboration,
        unix_now()?,
    )))
}

pub(crate) async fn create_repository_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<CreateRepositoryInviteRequest>,
) -> Result<Json<RepositoryInviteResponse>, ApiError> {
    let user = require_owner(&state, &headers, &owner, &repo_name).await?;
    let now = unix_now()?;
    let mutation = state
        .metadata
        .repositories()
        .create_repository_invite(scope_postgres::db::CreateRepositoryInviteMutation {
            owner,
            name: repo_name,
            owner_user: user,
            invited_email: input.email,
            permissions: input.permissions.into(),
            invite_id: random_token("repo_invite_", "failed to generate invite id")?,
            email_id: random_token("invite_email_", "failed to generate email id")?,
            now_unix: now,
        })
        .await?;
    let response = publish_committed_mutation(
        &state,
        map_committed_mutation(mutation, |(invite, email)| {
            repository_invite_response(&invite, email.as_ref(), now)
        }),
        RepoChangeReason::InviteUpdated,
    )
    .await;
    state.wakeups.invite_emails.notify_one();

    Ok(Json(response))
}

pub(crate) async fn create_repository_invite_email(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, invite_id)): Path<(String, String, String)>,
) -> Result<Json<RepositoryInviteResponse>, ApiError> {
    let user = require_owner(&state, &headers, &owner, &repo_name).await?;
    let now = unix_now()?;
    let mutation = state
        .metadata
        .repositories()
        .request_repository_invite_email(scope_postgres::db::RequestRepositoryInviteEmailCommand {
            owner,
            name: repo_name,
            owner_user_id: user.id,
            invite_id,
            email_id: random_token("invite_email_", "failed to generate email id")?,
            now_unix: now,
        })
        .await?;
    let response = publish_committed_mutation(
        &state,
        map_committed_mutation(mutation, |(invite, email)| {
            repository_invite_response(&invite, Some(&email), now)
        }),
        RepoChangeReason::InviteUpdated,
    )
    .await;
    state.wakeups.invite_emails.notify_one();

    Ok(Json(response))
}

pub(crate) async fn create_repository_invite_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, invite_id)): Path<(String, String, String)>,
) -> Result<Json<RepositoryInviteLinkResponse>, ApiError> {
    let user = require_owner(&state, &headers, &owner, &repo_name).await?;
    let (secret, link_hash) = generate_repository_invite_token()?;
    let invite_url = repository_invite_url(&secret)?;
    let mutation = state
        .metadata
        .repositories()
        .issue_repository_invite_link(scope_postgres::db::IssueRepositoryInviteLinkCommand {
            owner,
            name: repo_name,
            owner_user_id: user.id,
            invite_id,
            link_hash,
            now_unix: unix_now()?,
        })
        .await?;
    let response = publish_committed_mutation(
        &state,
        map_committed_mutation(mutation, |_| RepositoryInviteLinkResponse { invite_url }),
        RepoChangeReason::InviteUpdated,
    )
    .await;

    Ok(Json(response))
}

pub(crate) async fn update_repository_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, member_user_id)): Path<(String, String, String)>,
    Json(input): Json<UpdateRepositoryMemberRequest>,
) -> Result<Json<RepositoryMemberResponse>, ApiError> {
    let now_unix = unix_now()?;
    let user = require_owner(&state, &headers, &owner, &repo_name).await?;
    let member_user = state.metadata.repositories().user(&member_user_id).await?;
    let mutation = state
        .metadata
        .repositories()
        .update_repository_member_permissions(
            scope_postgres::db::UpdateRepositoryMemberPermissionsCommand {
                owner,
                name: repo_name,
                owner_user_id: user.id,
                member_user_id,
                permissions: input.permissions.into(),
                now_unix,
            },
        )
        .await?;
    let member = publish_committed_mutation(
        &state,
        map_committed_mutation(mutation, |member| {
            repository_member_response(&member, &member_user)
        }),
        RepoChangeReason::MemberPermissionsChanged,
    )
    .await;

    Ok(Json(member))
}

pub(crate) async fn delete_repository_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, invite_id)): Path<(String, String, String)>,
) -> Result<Json<RepositoryInviteResponse>, ApiError> {
    let user = require_owner(&state, &headers, &owner, &repo_name).await?;
    let now = unix_now()?;
    let mutation = state
        .metadata
        .repositories()
        .revoke_repository_invite(&owner, &repo_name, &user.id, &invite_id, now)
        .await?;
    let invite = publish_committed_mutation(
        &state,
        map_committed_mutation(mutation, |invite| {
            repository_invite_response(&invite, None, now)
        }),
        RepoChangeReason::InviteRevoked,
    )
    .await;

    Ok(Json(invite))
}

pub(crate) async fn delete_repository_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, member_user_id)): Path<(String, String, String)>,
) -> Result<Json<RepositoryMemberResponse>, ApiError> {
    let now_unix = unix_now()?;
    let user = require_owner(&state, &headers, &owner, &repo_name).await?;
    let member_user = state.metadata.repositories().user(&member_user_id).await?;
    let mutation = state
        .metadata
        .repositories()
        .remove_repository_member(&owner, &repo_name, &user.id, &member_user_id, now_unix)
        .await?;
    let member = publish_committed_mutation(
        &state,
        map_committed_mutation(mutation, |member| {
            repository_member_response(&member, &member_user)
        }),
        RepoChangeReason::MemberRemoved,
    )
    .await;

    Ok(Json(member))
}

pub(crate) async fn get_repository_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<Json<RepositoryInviteLandingResponse>, ApiError> {
    let viewer = optional_scope_user(&state, &headers).await?;
    let Some((repo, invite)) = state
        .metadata
        .repositories()
        .repository_invite_by_link_hash(&token_hash(&token))
        .await?
    else {
        return Ok(Json(RepositoryInviteLandingResponse::Invalid));
    };
    let landing = repository_invite_landing(&repo, &invite, viewer.as_ref(), unix_now()?);
    Ok(Json(repository_invite_landing_response(
        landing,
        &repo.record,
        &invite,
        viewer.as_ref(),
    )))
}

pub(crate) async fn accept_repository_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<Json<AcceptRepositoryInviteResponse>, ApiError> {
    let git_origin = state.git_public_url.to_string();
    let user = require_scope_user(&state, &headers).await?;
    let now = unix_now()?;
    let token_hash = token_hash(&token);
    let (repo, outcome) = accept_invite(&state, &token_hash, user.clone(), now).await?;
    let member = match outcome {
        AcceptRepositoryInviteOutcome::Accepted(member) => {
            state
                .publish_repo_change(
                    &repo.incarnation(),
                    repo.record.change_version,
                    RepoChangeReason::MemberAdded,
                )
                .await;
            member
        }
        AcceptRepositoryInviteOutcome::AlreadyAccepted(member) => member,
    };
    let open_request_count =
        open_request_count_for_access(&state, &repo.record.id, repo.access.clone(), &repo.views)
            .await?;
    let summary = repo_summary_for_access(
        &repo.record,
        repo.access.clone(),
        &repo.views,
        open_request_count,
        &git_origin,
    )
    .ok_or_else(|| ApiError::internal_message("accepted invite member cannot read repo"))?;
    Ok(Json(AcceptRepositoryInviteResponse {
        repo: summary,
        member: repository_member_response(&member, &user),
    }))
}

async fn require_owner(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
) -> Result<UserAccount, ApiError> {
    let user = require_scope_user(state, headers).await?;
    find_read_access(state, owner, repo_name, Some(&user.id))
        .await?
        .ensure_owner()?;
    Ok(user)
}

fn repository_invite_url(secret: &str) -> Result<String, ApiError> {
    let app_origin = public_app_origin("building repository invite URL")?;
    Ok(format!(
        "{}/invites/{secret}",
        app_origin.trim_end_matches('/')
    ))
}

async fn open_request_count_for_access(
    state: &AppState,
    repo_id: &str,
    access: RepositoryAccess,
    views: &scope_domain::views::Views,
) -> Result<usize, ApiError> {
    Ok(state
        .metadata
        .requests()
        .requests_by_repo_id(repo_id)
        .await?
        .into_iter()
        .filter(|request| request_counts_for_access(request, access.clone(), views))
        .count())
}

fn request_counts_for_access(
    request: &Request,
    access: RepositoryAccess,
    views: &scope_domain::views::Views,
) -> bool {
    request_policy(request, RequestViewer::new(access, None, false), views).counts_as_open
}
