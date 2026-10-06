use super::*;

pub mod projection_read_model {
    use super::*;
    use scope_domain::{history::HistoryCursor, projection::ProjectionCursor};

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_projection_read_models")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub audience: String,
        pub repo_version: i64,
        pub identity_version: i16,
        pub history_version: String,
        pub folded_commits: i64,
        pub folded_change_sets: i64,
        pub last_commit_id: Option<String>,
        pub last_change_set_id: Option<String>,
        pub projected_commits: i64,
        pub last_projected_id: Option<String>,
        pub head_oid: Option<String>,
        pub file_count: i64,
        pub visible_files: bool,
        pub history_entries: i64,
        pub last_history_entry_id: Option<String>,
        pub history_generation: String,
        pub views: Json,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct FoldPosition {
        pub commits: usize,
        pub change_sets: usize,
        pub last_commit_id: Option<String>,
        pub last_change_set_id: Option<String>,
    }

    impl Model {
        pub fn current(&self) -> bool {
            self.identity_version == scope_git::PROJECTION_IDENTITY_VERSION
                && self.history_version == scope_domain::history::HISTORY_GENERATION_VERSION
        }

        pub fn position(&self) -> Result<FoldPosition, PostgresError> {
            Ok(FoldPosition {
                commits: i64_to_usize(self.folded_commits, "folded commit count")?,
                change_sets: i64_to_usize(self.folded_change_sets, "folded change set count")?,
                last_commit_id: self.last_commit_id.clone(),
                last_change_set_id: self.last_change_set_id.clone(),
            })
        }

        pub fn projection_cursor(&self) -> Result<ProjectionCursor, PostgresError> {
            Ok(ProjectionCursor {
                commit_count: i64_to_usize(self.projected_commits, "projected commit count")?,
                last_projected_id: self.last_projected_id.clone(),
                views: decode_json(self.views.clone())?,
            })
        }

        pub fn history_cursor(&self) -> HistoryCursor {
            HistoryCursor {
                last_entry_id: self.last_history_entry_id.clone(),
                generation: self.history_generation.clone(),
            }
        }

        pub fn history_entries(&self) -> Result<usize, PostgresError> {
            i64_to_usize(self.history_entries, "history entry count")
        }
    }

    fn i64_to_usize(value: i64, label: &str) -> Result<usize, PostgresError> {
        usize::try_from(value)
            .map_err(|_| PostgresError::internal_message(format!("{label} cannot be negative")))
    }
}

pub mod projection_file {
    use super::*;
    use sha2::{Digest as _, Sha256};

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_projection_files")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub path_key: String,
        pub path: String,
        pub oid: String,
        pub visibility: String,
        pub sha256: String,
        pub object_key: String,
        pub size_bytes: i64,
        pub git_file_mode: String,
    }

    pub(crate) fn projection_file_path_key(path: &ScopePath) -> String {
        format!(
            "sha256:{}",
            hex::encode(Sha256::digest(path.as_str().as_bytes()))
        )
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn live(
            repo_id: &str,
            content: ProjectionViewFileContent,
        ) -> Result<Self, PostgresError> {
            if !content.file.tracked {
                return Err(PostgresError::internal_message(
                    "projection file content must be tracked",
                ));
            }
            if content.file.oid != content.blob.git_oid {
                return Err(PostgresError::internal_message(
                    "projection file and blob Git OIDs must match",
                ));
            }
            if !is_supported_git_file_mode(&content.blob.git_file_mode) {
                return Err(PostgresError::internal_message(
                    "projection file has unsupported Git mode",
                ));
            }
            let path_key = projection_file_path_key(&content.file.path);
            Ok(Self {
                repo_id: repo_id.to_string(),
                path_key,
                path: content.file.path.as_str().to_string(),
                oid: content.file.oid,
                visibility: content.file.label.as_str().to_string(),
                sha256: content.blob.sha256,
                object_key: serde_json::to_string(&content.blob.content_ref)
                    .map_err(PostgresError::internal)?,
                size_bytes: u64_to_i64(content.blob.size_bytes, "projection file size")?,
                git_file_mode: content.blob.git_file_mode,
            })
        }

        pub fn try_into_content(self) -> Result<ProjectionViewFileContent, PostgresError> {
            if !is_supported_git_file_mode(&self.git_file_mode) {
                return Err(PostgresError::internal_message(
                    "projection file has unsupported Git mode",
                ));
            }
            Ok(ProjectionViewFileContent {
                file: ProjectionViewFile {
                    path: ScopePath::parse(&self.path).map_err(PostgresError::internal)?,
                    oid: self.oid.clone(),
                    tracked: true,
                    label: ViewId::parse(&self.visibility).map_err(PostgresError::internal)?,
                },
                blob: SourceBlob {
                    content_ref: serde_json::from_str(&self.object_key)
                        .map_err(PostgresError::internal)?,
                    sha256: self.sha256,
                    git_oid: self.oid,
                    git_file_mode: self.git_file_mode,
                    size_bytes: i64_to_u64(self.size_bytes, "projection file size")?,
                },
            })
        }

        pub fn try_into_view(self) -> Result<ProjectionViewFile, PostgresError> {
            Ok(self.try_into_content()?.file)
        }
    }
}
