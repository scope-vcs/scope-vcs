use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0075_incremental_projection_read_models"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- Projection read models are folded forward from the commits
                -- and change sets appended since they were last built, so each
                -- view records where its fold stands. Files are stored once
                -- with their label; a view's files are the rows whose label it
                -- shows. History entry payloads are stored once per repository
                -- and indexed per view. Everything here is derived, so it is
                -- dropped and rebuilt on the next read or push.
                DROP TABLE scope_repository_history_entries;
                DROP TABLE scope_repository_history_views;
                DROP TABLE scope_projection_files;
                DROP TABLE scope_projection_read_models;

                CREATE TABLE scope_projection_read_models (
                    repo_id varchar NOT NULL REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    audience varchar NOT NULL,
                    repo_version bigint NOT NULL,
                    identity_version smallint NOT NULL,
                    history_version text NOT NULL,
                    folded_commits bigint NOT NULL,
                    folded_change_sets bigint NOT NULL,
                    last_commit_id varchar,
                    last_change_set_id varchar,
                    projected_commits bigint NOT NULL,
                    last_projected_id varchar,
                    head_oid varchar,
                    file_count bigint NOT NULL,
                    visible_files boolean NOT NULL,
                    history_entries bigint NOT NULL,
                    last_history_entry_id text,
                    history_generation text NOT NULL,
                    PRIMARY KEY (repo_id, audience),
                    CONSTRAINT scope_projection_read_model_values CHECK (
                        audience IN ('private', 'public')
                        AND repo_version >= 0
                        AND folded_commits >= 0
                        AND folded_change_sets >= 0
                        AND (last_commit_id IS NULL) = (folded_commits = 0)
                        AND (last_change_set_id IS NULL) = (folded_change_sets = 0)
                        AND projected_commits >= 0
                        AND (last_projected_id IS NULL) = (projected_commits = 0)
                        AND (head_oid IS NULL) = (projected_commits = 0)
                        AND (head_oid IS NULL OR head_oid ~ '^[0-9a-f]{40}$')
                        AND file_count >= 0
                        AND history_entries >= 0
                        AND (last_history_entry_id IS NULL) = (history_entries = 0)
                    )
                );

                CREATE TABLE scope_projection_files (
                    repo_id varchar NOT NULL REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    path_key varchar NOT NULL,
                    path varchar NOT NULL,
                    oid varchar NOT NULL,
                    visibility varchar NOT NULL,
                    object_key varchar NOT NULL,
                    sha256 varchar NOT NULL,
                    size_bytes bigint NOT NULL,
                    git_file_mode varchar NOT NULL,
                    PRIMARY KEY (repo_id, path_key),
                    CONSTRAINT scope_projection_file_values CHECK (
                        visibility IN ('Public', 'Private')
                        AND size_bytes >= 0
                        AND git_file_mode IN ('100644', '100755')
                    )
                );

                CREATE TABLE scope_repository_history_payloads (
                    repo_id varchar NOT NULL REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    payload_hash text NOT NULL,
                    payload jsonb NOT NULL,
                    PRIMARY KEY (repo_id, payload_hash)
                );

                CREATE TABLE scope_repository_history_entries (
                    repo_id varchar NOT NULL,
                    audience varchar NOT NULL,
                    position bigint NOT NULL CHECK (position >= 0),
                    source_id text NOT NULL,
                    payload_hash text NOT NULL,
                    PRIMARY KEY (repo_id, audience, position),
                    UNIQUE (repo_id, audience, source_id),
                    FOREIGN KEY (repo_id, audience)
                        REFERENCES scope_projection_read_models(repo_id, audience) ON DELETE CASCADE,
                    FOREIGN KEY (repo_id, payload_hash)
                        REFERENCES scope_repository_history_payloads(repo_id, payload_hash)
                );
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Incremental projection read models are forward-only".into(),
        ))
    }
}
