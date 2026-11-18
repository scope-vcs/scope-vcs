use super::{
    RepositoryStore, acquire_aggregate_lock,
    cleanup_queue::queue::queue_pending_source_blob_deletion_rows,
    entities,
    git_compaction::schedule_git_compaction,
    git_segments::register_git_pool_pack,
    github_pushes::queue_main_publication,
    landing_files::apply_repository_landing_file_mutation,
    repository_from_model,
    repository_rows::save_repository_delta,
    request_merge::{LandedRequestCompletion, complete_main_push},
    request_rows::open_request_counts_by_view,
};
use sea_orm::{DatabaseTransaction, EntityTrait, TransactionTrait};
use std::{collections::BTreeMap, fmt};
use {
    crate::error::PostgresError,
    scope_domain::content::SourceBlob,
    scope_domain::repository::{Repository, git::GitPoolPack, repo_id},
    scope_domain::views::ViewId,
    scope_domain::{error::DomainError, landing_file::RepositoryLandingFileMutation},
};

#[derive(Debug)]
pub enum RepositoryMutationError {
    Behavior(DomainError),
    Persistence(PostgresError),
}

impl fmt::Display for RepositoryMutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Behavior(error) => error.fmt(formatter),
            Self::Persistence(error) => formatter.write_str(&error.message),
        }
    }
}

impl std::error::Error for RepositoryMutationError {}

impl From<DomainError> for RepositoryMutationError {
    fn from(error: DomainError) -> Self {
        Self::Behavior(error)
    }
}

impl From<PostgresError> for RepositoryMutationError {
    fn from(error: PostgresError) -> Self {
        Self::Persistence(error)
    }
}

pub struct RepositoryMutationResult<R> {
    pub result: R,
    pub completed_landed_requests: usize,
    pub queued_main_publication: bool,
}

pub struct RepositoryMutation<R> {
    pub result: R,
    pub orphan_objects: Vec<SourceBlob>,
    pub landing_file_mutation: RepositoryLandingFileMutation,
    pub landed_requests: Option<LandedRequestCompletion>,
    pub main_pack: Option<GitPoolPack>,
}

impl<R> RepositoryMutation<R> {
    pub fn new(result: R) -> Self {
        Self {
            result,
            orphan_objects: Vec::new(),
            landing_file_mutation: RepositoryLandingFileMutation::Unchanged,
            landed_requests: None,
            main_pack: None,
        }
    }

    pub fn with_landing_file_mutation(
        result: R,
        landing_file_mutation: RepositoryLandingFileMutation,
    ) -> Self {
        Self {
            result,
            orphan_objects: Vec::new(),
            landing_file_mutation,
            landed_requests: None,
            main_pack: None,
        }
    }
}

pub struct RepositoryTransaction {
    tx: DatabaseTransaction,
    repository: Repository,
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "begin_repository"))]
    pub async fn begin_repository(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<RepositoryTransaction, PostgresError> {
        let repo_id = repo_id(owner, name);
        let tx = self.db.begin().await?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let repo = entities::repository::Entity::find_by_id(&repo_id)
            .one(&tx)
            .await?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        let repository = repository_from_model(&tx, repo).await?;
        Ok(RepositoryTransaction { tx, repository })
    }
}

impl RepositoryTransaction {
    pub fn repository(&self) -> &Repository {
        &self.repository
    }

    pub async fn open_request_counts_by_view(
        &self,
    ) -> Result<BTreeMap<ViewId, usize>, PostgresError> {
        open_request_counts_by_view(&self.tx, &self.repository.record.id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "save_repository"))]
    pub async fn save<R>(
        self,
        repo: Repository,
        mutation: RepositoryMutation<R>,
        now_unix: u64,
    ) -> Result<RepositoryMutationResult<R>, PostgresError> {
        let Self {
            tx,
            repository: before,
        } = self;

        let advanced_head = repo.git_head.as_ref().filter(|head| {
            before.git_head.as_ref().map(|head| head.frontier()) != Some(head.frontier())
        });
        match (advanced_head, &mutation.main_pack) {
            (None, None) => {}
            (Some(head), Some(pack)) if pack.head_oid == head.head_oid => {}
            _ => {
                return Err(PostgresError::internal_message(
                    "a repository mutation must register exactly one main pack for each new Git head",
                ));
            }
        }
        save_repository_delta(&tx, &before, &repo, now_unix).await?;
        if let (Some(pack), Some(head)) = (&mutation.main_pack, &repo.git_head) {
            register_git_pool_pack(&tx, &repo.record.id, pack, now_unix).await?;
            schedule_git_compaction(&tx, &repo.record.id, head.push_sequence, now_unix).await?;
        }
        apply_repository_landing_file_mutation(
            &tx,
            &repo.record.id,
            mutation.landing_file_mutation,
        )
        .await?;
        let completed_landed_requests = if let Some(completion) = mutation.landed_requests {
            let head = repo.git_head.as_ref().ok_or_else(|| {
                PostgresError::internal_message("landed requests require an accepted Git head")
            })?;
            complete_main_push(&tx, &repo.record.id, &head.head_oid, completion, now_unix).await?
        } else {
            0
        };
        let changed_main = repo.git_head.as_ref().filter(|head| {
            before.git_head.as_ref().map(|before| &before.head_oid) != Some(&head.head_oid)
        });
        let queued_main_publication = match changed_main {
            Some(head) => {
                queue_main_publication(&tx, &repo.record.id, &head.head_oid, now_unix).await?
            }
            None => false,
        };
        queue_pending_source_blob_deletion_rows(&tx, mutation.orphan_objects, now_unix).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RepositoryMutationResult {
            result: mutation.result,
            completed_landed_requests,
            queued_main_publication,
        })
    }
}

#[cfg(test)]
impl RepositoryStore {
    pub(crate) async fn mutate_repository<R>(
        &self,
        owner: &str,
        name: &str,
        now_unix: u64,
        decide: impl FnOnce(&mut Repository) -> Result<RepositoryMutation<R>, DomainError>,
    ) -> Result<RepositoryMutationResult<R>, RepositoryMutationError> {
        let tx = self.begin_repository(owner, name).await?;
        let mut repo = tx.repository().clone();
        let mutation = decide(&mut repo)?;
        Ok(tx.save(repo, mutation, now_unix).await?)
    }
}
