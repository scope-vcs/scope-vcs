use super::{commit_belongs_to_revision, commits::request_commit_is_visible_to};
use crate::{
    error::ApiError, git::request_refs::with_request_revision_store_repo, state::AppState,
};
use scope_domain::{
    policy::Policy,
    repository::{RepositoryIncarnation, access::RepositoryAccess},
    requests::Request,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct RequestRevisionCommitVisibility<'a> {
    state: &'a AppState,
    incarnation: &'a RepositoryIncarnation,
    policy: &'a Policy,
    access: RepositoryAccess,
    request: &'a Request,
}

impl<'a> RequestRevisionCommitVisibility<'a> {
    pub(crate) fn new(
        state: &'a AppState,
        incarnation: &'a RepositoryIncarnation,
        policy: &'a Policy,
        access: RepositoryAccess,
        request: &'a Request,
    ) -> Self {
        Self {
            state,
            incarnation,
            policy,
            access,
            request,
        }
    }

    pub(crate) async fn visible_commits(
        &self,
        commits_by_revision: &BTreeMap<String, BTreeSet<String>>,
    ) -> BTreeSet<(String, String)> {
        let mut visible = BTreeSet::new();
        for (revision_id, commit_oids) in commits_by_revision {
            let result = self
                .visible_commits_in_revision(revision_id, commit_oids)
                .await;
            match result {
                Ok(commit_oids) => visible.extend(
                    commit_oids
                        .into_iter()
                        .map(|commit_oid| (revision_id.clone(), commit_oid)),
                ),
                Err(error) => tracing::warn!(
                    request_id = %self.request.id,
                    revision_id,
                    error = ?error,
                    "redacting discussion anchors because request revision inspection failed"
                ),
            }
        }
        visible
    }

    async fn visible_commits_in_revision(
        &self,
        revision_id: &str,
        commit_oids: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, ApiError> {
        let Some(revision) = self
            .state
            .metadata
            .requests()
            .request_revision(&self.request.id, revision_id)
            .await?
        else {
            return Ok(BTreeSet::new());
        };
        let policy = self.policy.clone();
        let access = self.access;
        let commit_oids = commit_oids.clone();
        with_request_revision_store_repo(
            self.state,
            self.incarnation,
            self.request,
            &revision,
            move |raw_repo, revision| {
                let mut visible = BTreeSet::new();
                for commit_oid in &commit_oids {
                    if commit_belongs_to_revision(raw_repo, revision, commit_oid)?
                        && request_commit_is_visible_to(raw_repo, &policy, access, commit_oid)?
                    {
                        visible.insert(commit_oid.clone());
                    }
                }
                Ok(visible)
            },
        )
        .await
    }
}
