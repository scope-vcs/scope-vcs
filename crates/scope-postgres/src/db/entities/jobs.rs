use super::*;

pub(crate) mod git_compaction_job {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_git_compaction_jobs")]
    pub(crate) struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub target_sequence: i64,
        pub attempts: i32,
        pub next_run_at_unix: i64,
        pub lease_generation: Option<String>,
        pub lease_owner: Option<String>,
        pub lease_expires_at_unix: Option<i64>,
        pub last_error: Option<String>,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
        pub traceparent: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub(crate) enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod metadata_lock {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_metadata_locks")]
    pub(crate) struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub key: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub(crate) enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
pub(crate) mod repo_storage_cleanup_job {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repo_storage_cleanup_jobs")]
    pub(crate) struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        pub generation: String,
        pub owner_handle: String,
        pub repo_name: String,
        pub incarnation_id: String,
        pub attempts: i32,
        pub next_run_at_unix: i64,
        pub last_error: Option<String>,
        pub completed_at_unix: Option<i64>,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub(crate) enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(
            cleanup: &RepoStorageCleanup,
            generation: String,
            now_unix: u64,
        ) -> Result<Self, PostgresError> {
            let repo_id =
                scope_domain::repository::repo_id(&cleanup.owner_handle, &cleanup.repo_name);
            let now_unix = u64_to_i64(now_unix, "cleanup creation time")?;
            Ok(Self {
                repo_id,
                generation,
                owner_handle: cleanup.owner_handle.clone(),
                repo_name: cleanup.repo_name.clone(),
                incarnation_id: cleanup.incarnation.incarnation_id().to_string(),
                attempts: 0,
                next_run_at_unix: now_unix,
                last_error: None,
                completed_at_unix: None,
                created_at_unix: now_unix,
                updated_at_unix: now_unix,
            })
        }

        pub(crate) fn try_into_domain(self) -> Result<RepoStorageCleanup, PostgresError> {
            Ok(RepoStorageCleanup {
                owner_handle: self.owner_handle,
                repo_name: self.repo_name,
                incarnation: scope_domain::repository::RepositoryIncarnation::new(
                    self.repo_id,
                    self.incarnation_id,
                )
                .map_err(PostgresError::internal)?,
            })
        }
    }
}
pub(crate) mod source_blob_cleanup_job {
    use super::*;
    use crate::db::object_references::{decode_content_ref, encode_content_ref};

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_orphan_object_jobs")]
    pub(crate) struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub object_key: String,
        pub generation: String,
        pub sha256: String,
        pub git_oid: String,
        pub size_bytes: i64,
        pub attempts: i32,
        pub next_run_at_unix: i64,
        pub last_error: Option<String>,
        pub completed_at_unix: Option<i64>,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
        pub traceparent: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub(crate) enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub(crate) fn from_domain(
            blob: &SourceBlob,
            generation: String,
            now_unix: u64,
        ) -> Result<Self, PostgresError> {
            let now_unix = u64_to_i64(now_unix, "cleanup creation time")?;
            Ok(Self {
                object_key: encode_content_ref(&blob.content_ref)?,
                generation,
                sha256: blob.sha256.clone(),
                git_oid: blob.git_oid.clone(),
                size_bytes: u64_to_i64(blob.size_bytes, "source blob size")?,
                attempts: 0,
                next_run_at_unix: now_unix,
                last_error: None,
                completed_at_unix: None,
                created_at_unix: now_unix,
                updated_at_unix: now_unix,
                traceparent: QueuedTrace::current().into_column(),
            })
        }

        pub(crate) fn try_into_domain(self) -> Result<SourceBlob, PostgresError> {
            Ok(SourceBlob {
                content_ref: decode_content_ref(&self.object_key)?,
                sha256: self.sha256,
                git_oid: self.git_oid,
                git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                size_bytes: i64_to_u64(self.size_bytes, "source blob size")?,
            })
        }
    }
}
