use super::commit_is_fully_visible;
use crate::{
    error::ApiError, git::request_refs::with_request_revision_store_repo, state::AppState,
};
use scope_domain::{
    policy::Policy,
    repository::{RepositoryIncarnation, access::RepositoryAccess},
    requests::Request,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) async fn visible_revision_commits(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    policy: &Policy,
    access: RepositoryAccess,
    request: &Request,
    commits_by_revision: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<(String, String)> {
    let mut visible = BTreeSet::new();
    for (revision_id, commit_oids) in commits_by_revision {
        match visible_commits_in_revision(
            state,
            incarnation,
            policy,
            access,
            request,
            revision_id,
            commit_oids,
        )
        .await
        {
            Ok(commit_oids) => visible.extend(
                commit_oids
                    .into_iter()
                    .map(|oid| (revision_id.clone(), oid)),
            ),
            Err(error) => tracing::warn!(
                request_id = %request.id,
                revision_id,
                error = ?error,
                "redacting discussion anchors because request revision inspection failed"
            ),
        }
    }
    visible
}

async fn visible_commits_in_revision(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    policy: &Policy,
    access: RepositoryAccess,
    request: &Request,
    revision_id: &str,
    commit_oids: &BTreeSet<String>,
) -> Result<BTreeSet<String>, ApiError> {
    let Some(revision) = state
        .metadata
        .requests()
        .request_revision(&request.id, revision_id)
        .await?
    else {
        return Ok(BTreeSet::new());
    };
    let policy = policy.clone();
    let commit_oids = commit_oids.clone();
    with_request_revision_store_repo(
        state,
        incarnation,
        request,
        &revision,
        move |raw_repo, revision| {
            let mut visible = BTreeSet::new();
            for oid in commit_oids {
                if commit_is_fully_visible(raw_repo, &policy, access, revision, &oid)? {
                    visible.insert(oid);
                }
            }
            Ok(visible)
        },
    )
    .await
}
