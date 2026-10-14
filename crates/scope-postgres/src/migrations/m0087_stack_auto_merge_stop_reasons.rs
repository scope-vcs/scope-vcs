use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0087_stack_auto_merge_stop_reasons"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
            ALTER TABLE scope_request_auto_merge_intents
                DROP CONSTRAINT scope_request_auto_merge_values,
                ADD CONSTRAINT scope_request_auto_merge_values CHECK (
                        length(btrim(id)) > 0 AND
                        length(btrim(repo_id)) > 0 AND
                        length(btrim(repository_incarnation_id)) > 0 AND
                        length(btrim(request_id)) > 0 AND
                        length(btrim(revision_id)) > 0 AND
                        length(head_oid) = 40 AND
                        length(btrim(actor_user_id)) > 0 AND
                        status IN ('Active', 'Cancelled', 'Stopped', 'Fulfilled') AND
                        reason IN (
                            'RequestChanged', 'RequestClosed', 'AccessRevoked',
                            'ChecksFailed', 'ChecksConfigurationError', 'MergeConflict',
                            'RequestBranchMissing', 'ParentRewritten', 'ParentClosed', 'AncestorBlocked'
                        ) AND
                        ((status = 'Stopped') = (reason IS NOT NULL)) AND
                        created_position > 0 AND
                        ((claim_token IS NULL) = (claim_expires_at_unix IS NULL)) AND
                        (status = 'Active' OR claim_token IS NULL) AND
                        attempt >= 0 AND next_attempt_at_unix >= 0 AND
                        (last_error IS NULL OR octet_length(last_error) BETWEEN 1 AND 8192) AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix
                );
        "#).await?;
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Migration(
            "stack auto-merge stop reasons are forward-only".into(),
        ))
    }
}
