use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0063_account_deletion"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- Work in other people's repositories outlives its author's
                -- account and is shown as a deleted user's.
                ALTER TABLE scope_requests
                    ALTER COLUMN author_user_id DROP NOT NULL,
                    DROP CONSTRAINT fk_scope_requests_author,
                    ADD CONSTRAINT fk_scope_requests_author FOREIGN KEY (author_user_id)
                        REFERENCES scope_users(id) ON DELETE SET NULL,
                    DROP CONSTRAINT fk_scope_requests_closer,
                    ADD CONSTRAINT fk_scope_requests_closer FOREIGN KEY (closed_by_user_id)
                        REFERENCES scope_users(id) ON DELETE SET NULL,
                    DROP CONSTRAINT fk_scope_requests_merger,
                    ADD CONSTRAINT fk_scope_requests_merger FOREIGN KEY (merged_by_user_id)
                        REFERENCES scope_users(id) ON DELETE SET NULL,
                    -- A closed or merged request keeps its state after the
                    -- account that closed or merged it is gone.
                    DROP CONSTRAINT scope_request_merge_coherence,
                    ADD CONSTRAINT scope_request_merge_coherence CHECK (
                        (merged_at_unix IS NULL AND merged_by_user_id IS NULL AND
                            merged_head_oid IS NULL AND merged_main_oid IS NULL) OR
                        (submitted_at_unix IS NOT NULL AND merged_at_unix IS NOT NULL AND
                            merged_head_oid IS NOT NULL AND length(merged_head_oid) > 0 AND
                            merged_main_oid IS NOT NULL AND length(merged_main_oid) > 0)
                    ),
                    DROP CONSTRAINT scope_request_submission_coherence,
                    ADD CONSTRAINT scope_request_submission_coherence CHECK (
                        (closed_at_unix IS NULL OR merged_at_unix IS NULL) AND
                        (closed_at_unix IS NOT NULL OR closed_by_user_id IS NULL) AND
                        ((submitted_at_unix IS NULL AND closed_at_unix IS NULL AND
                            merged_at_unix IS NULL) OR
                         (submitted_at_unix IS NOT NULL AND
                            (closed_at_unix IS NULL OR closed_at_unix >= submitted_at_unix) AND
                            (merged_at_unix IS NULL OR merged_at_unix >= submitted_at_unix)))
                    );

                ALTER TABLE scope_request_events
                    ALTER COLUMN actor_user_id DROP NOT NULL,
                    DROP CONSTRAINT fk_scope_request_events_actor,
                    ADD CONSTRAINT fk_scope_request_events_actor FOREIGN KEY (actor_user_id)
                        REFERENCES scope_users(id) ON DELETE SET NULL;

                ALTER TABLE scope_request_revisions
                    ALTER COLUMN actor_user_id DROP NOT NULL,
                    DROP CONSTRAINT fk_scope_request_revisions_actor,
                    ADD CONSTRAINT fk_scope_request_revisions_actor FOREIGN KEY (actor_user_id)
                        REFERENCES scope_users(id) ON DELETE SET NULL;

                ALTER TABLE scope_request_discussions
                    ALTER COLUMN author_user_id DROP NOT NULL,
                    DROP CONSTRAINT fk_scope_request_discussions_author,
                    ADD CONSTRAINT fk_scope_request_discussions_author FOREIGN KEY (author_user_id)
                        REFERENCES scope_users(id) ON DELETE SET NULL;

                ALTER TABLE scope_request_discussion_replies
                    ALTER COLUMN author_user_id DROP NOT NULL,
                    DROP CONSTRAINT fk_scope_request_discussion_replies_author,
                    ADD CONSTRAINT fk_scope_request_discussion_replies_author
                        FOREIGN KEY (author_user_id)
                        REFERENCES scope_users(id) ON DELETE SET NULL;

                ALTER TABLE scope_request_invitees
                    ALTER COLUMN invited_by_user_id DROP NOT NULL,
                    DROP CONSTRAINT fk_scope_request_invitees_inviter,
                    ADD CONSTRAINT fk_scope_request_invitees_inviter
                        FOREIGN KEY (invited_by_user_id)
                        REFERENCES scope_users(id) ON DELETE SET NULL;

                ALTER TABLE scope_runs
                    DROP CONSTRAINT fk_scope_runs_requester,
                    ADD CONSTRAINT fk_scope_runs_requester FOREIGN KEY (requested_by_user_id)
                        REFERENCES scope_users(id) ON DELETE SET NULL;

                -- Ratings, claims and auto-merge authorizations belong to the
                -- account. Account deletion stops active authorizations first,
                -- so the request activity records why they ended.
                ALTER TABLE scope_request_ratings
                    DROP CONSTRAINT fk_scope_request_ratings_rater,
                    ADD CONSTRAINT fk_scope_request_ratings_rater FOREIGN KEY (rater_user_id)
                        REFERENCES scope_users(id) ON DELETE CASCADE,
                    DROP CONSTRAINT fk_scope_request_ratings_subject,
                    ADD CONSTRAINT fk_scope_request_ratings_subject FOREIGN KEY (subject_user_id)
                        REFERENCES scope_users(id) ON DELETE CASCADE;

                ALTER TABLE scope_request_claims
                    DROP CONSTRAINT scope_request_claims_claimer_user_id_fkey,
                    ADD CONSTRAINT scope_request_claims_claimer_user_id_fkey
                        FOREIGN KEY (claimer_user_id)
                        REFERENCES scope_users(id) ON DELETE CASCADE;

                ALTER TABLE scope_request_auto_merge_intents
                    DROP CONSTRAINT scope_request_auto_merge_intents_actor_user_id_fkey,
                    ADD CONSTRAINT scope_request_auto_merge_intents_actor_user_id_fkey
                        FOREIGN KEY (actor_user_id)
                        REFERENCES scope_users(id) ON DELETE CASCADE;

                -- Owned repositories leave through repository deletion, which
                -- queues their storage cleanup. A cascade would orphan it.
                ALTER TABLE scope_repositories
                    DROP CONSTRAINT fk_scope_repositories_owner,
                    ADD CONSTRAINT fk_scope_repositories_owner FOREIGN KEY (owner_user_id)
                        REFERENCES scope_users(id) ON DELETE RESTRICT;

                -- Clerk users of deleted accounts, deleted from Clerk after the
                -- Scope deletion commits. A row is removed once Clerk confirms.
                CREATE TABLE scope_clerk_user_deletions (
                    clerk_user_id varchar PRIMARY KEY,
                    attempts integer NOT NULL DEFAULT 0,
                    next_attempt_at_unix bigint NOT NULL,
                    claim_token varchar,
                    claim_expires_at_unix bigint,
                    last_error text,
                    created_at_unix bigint NOT NULL,
                    CONSTRAINT scope_clerk_user_deletion_values CHECK (
                        length(btrim(clerk_user_id)) > 0 AND
                        attempts >= 0 AND next_attempt_at_unix >= 0 AND
                        ((claim_token IS NULL) = (claim_expires_at_unix IS NULL)) AND
                        (last_error IS NULL OR octet_length(last_error) BETWEEN 1 AND 8192) AND
                        created_at_unix >= 0
                    )
                );

                CREATE INDEX idx_scope_clerk_user_deletions_due
                    ON scope_clerk_user_deletions(next_attempt_at_unix, clerk_user_id);
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Account deletion is forward-only".into()))
    }
}
