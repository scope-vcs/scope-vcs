use super::{
    account::UserAccount,
    content::SourceBlob,
    policy::Visibility,
    repository::{
        CatalogError, Repository,
        credentials::{FirstPushToken, GitPushToken},
    },
    reviewed_updates::error::ReviewedUpdateError,
};
use crate::error::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoStorageCleanup {
    pub owner_handle: String,
    pub repo_name: String,
    pub incarnation: crate::repository::RepositoryIncarnation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoEffect {
    DeleteRepoStorage(RepoStorageCleanup),
    DeleteSourceBlobs(Vec<SourceBlob>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoEffects {
    effects: Vec<RepoEffect>,
}

impl RepoEffects {
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &RepoEffect> {
        self.effects.iter()
    }

    fn delete_repo_storage(&mut self, cleanup: RepoStorageCleanup) {
        self.effects.push(RepoEffect::DeleteRepoStorage(cleanup));
    }

    fn delete_source_blobs(&mut self, blobs: impl IntoIterator<Item = SourceBlob>) {
        let blobs = blobs.into_iter().collect::<Vec<_>>();
        if !blobs.is_empty() {
            self.effects.push(RepoEffect::DeleteSourceBlobs(blobs));
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoMutation<T> {
    pub result: T,
    pub effects: RepoEffects,
}

impl<T> RepoMutation<T> {
    fn new(result: T) -> Self {
        Self {
            result,
            effects: RepoEffects::default(),
        }
    }

    fn with_effects(result: T, effects: RepoEffects) -> Self {
        Self { result, effects }
    }
}

pub fn ensure_repo_owner(repo: &Repository, user_id: &str) -> Result<(), DomainError> {
    if !repo.is_owner_user(user_id) {
        return Err(DomainError::forbidden("owner role required"));
    }
    Ok(())
}

pub fn ensure_repo_member(repo: &Repository, user_id: &str) -> Result<(), DomainError> {
    if repo.is_owner_user(user_id) || repo.member_for_user(user_id).is_some() {
        Ok(())
    } else {
        Err(DomainError::forbidden("repo membership required"))
    }
}

pub fn ensure_repo_delete_owner(
    repo: &Repository,
    user_id: &str,
    owner: &str,
    name: &str,
) -> Result<(), DomainError> {
    match ensure_repo_owner(repo, user_id) {
        Ok(()) => Ok(()),
        Err(_) => Err(hidden_repo_not_found(owner, name)),
    }
}

pub fn hidden_repo_not_found(owner: &str, name: &str) -> DomainError {
    DomainError::not_found(format!("repo {owner}/{name} not found"))
}

pub fn secretless_first_push_token(mut token: FirstPushToken) -> FirstPushToken {
    token.secret = None;
    token
}

pub fn catalog_error(error: CatalogError) -> DomainError {
    match error {
        CatalogError::InvalidRepositoryName(message)
        | CatalogError::InvalidRepositoryIdentity(message) => DomainError::invalid_input(message),
    }
}

pub fn reviewed_update_domain_error(error: ReviewedUpdateError) -> DomainError {
    match error {
        ReviewedUpdateError::BadRequest(message) => DomainError::invalid_input(message),
        ReviewedUpdateError::Conflict(message) => DomainError::conflict(message),
        ReviewedUpdateError::InvalidPolicy(error) => DomainError::invalid_input(error),
    }
}

pub fn create_repo(
    owner: &UserAccount,
    name: &str,
    default_visibility: Visibility,
    first_push_token: FirstPushToken,
    git_push_token: GitPushToken,
    incarnation_id: impl Into<String>,
) -> Result<RepoMutation<Repository>, DomainError> {
    let mut repo =
        Repository::new(owner, name, default_visibility, incarnation_id).map_err(catalog_error)?;
    repo.first_push_token = Some(secretless_first_push_token(first_push_token));
    repo.git_push_token = Some(git_push_token);
    Ok(RepoMutation::new(repo))
}

pub fn delete_repo(
    repo: &Repository,
    user_id: &str,
    owner: &str,
    name: &str,
) -> Result<RepoMutation<String>, DomainError> {
    ensure_repo_delete_owner(repo, user_id, owner, name)?;
    let mut effects = RepoEffects::default();
    effects.delete_repo_storage(RepoStorageCleanup {
        owner_handle: owner.to_string(),
        repo_name: name.to_string(),
        incarnation: repo.incarnation(),
    });
    effects.delete_source_blobs(repo.source_blobs());
    Ok(RepoMutation::with_effects(repo.record.id.clone(), effects))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        account::UserAccount,
        content::DEFAULT_GIT_FILE_MODE,
        policy::{ScopePath, Visibility},
        repository::git::GitHead,
    };

    #[test]
    fn deleting_repo_returns_storage_and_source_blob_cleanup_effects() {
        let owner = test_owner();
        let mut repo = Repository::new(&owner, "repo", Visibility::Private, "repoi_test").unwrap();
        let snapshot = source_blob("live-snapshot");
        repo.git_head = Some(GitHead::new(snapshot.git_oid.clone(), 1, 1));
        repo.graph.commits.push(crate::projection::LogicalCommit {
            occurred_at_unix: None,
            id: "commit-1".into(),
            origin: crate::projection::LogicalCommitOrigin::CanonicalPush {
                source_head_oid: snapshot.git_oid.clone(),
            },
            author_id: owner.id.clone(),
            message: "initial".into(),
            changes: vec![crate::projection::FileChange {
                path: ScopePath::parse("/README.md").unwrap(),
                old_content: None,
                new_content: Some(snapshot.clone()),
                visibility: Visibility::Private,
            }],
        });

        let mutation = delete_repo(&repo, &owner.id, &owner.handle, &repo.record.name).unwrap();

        assert_eq!(mutation.result, repo.record.id);
        assert_eq!(
            mutation.effects,
            RepoEffects {
                effects: vec![
                    RepoEffect::DeleteRepoStorage(RepoStorageCleanup {
                        owner_handle: owner.handle,
                        repo_name: "repo".to_string(),
                        incarnation: repo.incarnation(),
                    }),
                    RepoEffect::DeleteSourceBlobs(vec![snapshot]),
                ],
            }
        );
    }

    fn test_owner() -> UserAccount {
        UserAccount {
            id: "owner-id".to_string(),
            handle: "owner".to_string(),
            email: "owner@example.com".to_string(),
            email_verified: true,
        }
    }

    fn source_blob(label: &str) -> SourceBlob {
        SourceBlob {
            content_ref: crate::content_ref::ContentRef::blob_sha256(label),
            sha256: format!("sha256-{label}"),
            git_oid: format!("oid-{label}"),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: label.len() as u64,
        }
    }
}
