use crate::{
    error::ApiError,
    git::request_refs::with_request_revision_store_repo,
    state::AppState,
    use_cases::request_revision_inspection::{
        commit_belongs_to_revision, request_commit_visible_paths,
    },
};
use scope_domain::{
    policy::Policy,
    repository::{
        RepositoryIncarnation,
        access::{RepositoryAccess, RepositoryAccessContext},
    },
    requests::{Request, RequestDiscussionAnchor},
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) async fn visible_commits<'a>(
    state: &AppState,
    repo: &RepositoryAccessContext,
    request: &Request,
    anchors: impl IntoIterator<Item = &'a RequestDiscussionAnchor>,
) -> BTreeSet<(String, String)> {
    let commits_by_revision = anchored_commits_by_revision(anchors);
    if commits_by_revision.is_empty() {
        return BTreeSet::new();
    }
    if repo.access.can_read_private_files {
        return flatten(commits_by_revision);
    }
    let policy = match state.metadata.repositories().repository_policy(repo).await {
        Ok(policy) => policy,
        Err(error) => {
            tracing::warn!(
                request_id = %request.id,
                error = ?error,
                "redacting discussion anchors because the repository policy is unavailable"
            );
            return BTreeSet::new();
        }
    };
    let incarnation = repo.incarnation();
    let mut visible = BTreeSet::new();
    for (revision_id, commit_oids) in commits_by_revision {
        let result = visible_commits_in_revision(
            state,
            &incarnation,
            &policy,
            repo.access,
            request,
            &revision_id,
            commit_oids,
        )
        .await;
        match result {
            Ok(commit_oids) => visible.extend(
                commit_oids
                    .into_iter()
                    .map(|commit_oid| (revision_id.clone(), commit_oid)),
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

fn anchored_commits_by_revision<'a>(
    anchors: impl IntoIterator<Item = &'a RequestDiscussionAnchor>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut commits_by_revision = BTreeMap::<String, BTreeSet<String>>::new();
    for anchor in anchors {
        if let Some(commit_oid) = &anchor.commit_oid {
            commits_by_revision
                .entry(anchor.revision_id.clone())
                .or_default()
                .insert(commit_oid.clone());
        }
    }
    commits_by_revision
}

fn flatten(commits_by_revision: BTreeMap<String, BTreeSet<String>>) -> BTreeSet<(String, String)> {
    commits_by_revision
        .into_iter()
        .flat_map(|(revision_id, commit_oids)| {
            commit_oids
                .into_iter()
                .map(move |commit_oid| (revision_id.clone(), commit_oid))
        })
        .collect()
}

async fn visible_commits_in_revision(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    policy: &Policy,
    access: RepositoryAccess,
    request: &Request,
    revision_id: &str,
    commit_oids: BTreeSet<String>,
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
    with_request_revision_store_repo(
        state,
        incarnation,
        request,
        &revision,
        move |raw_repo, revision| {
            let mut visible = BTreeSet::new();
            for commit_oid in &commit_oids {
                if !commit_belongs_to_revision(raw_repo, revision, commit_oid)? {
                    continue;
                }
                let (_, hidden) =
                    request_commit_visible_paths(raw_repo, &policy, access, commit_oid)?;
                if !hidden {
                    visible.insert(commit_oid.clone());
                }
            }
            Ok(visible)
        },
    )
    .await
}
