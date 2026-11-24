use super::super::{
    RepositoryStore, RequestStore, acquire_aggregate_lock,
    entities::decode_enum,
    generated_ids::{GeneratedIdKind, generate_id},
    github::connections::repository_github_connection,
    github::import_queue::GitHubImportQueue,
    github::read_queue::GitHubJobOutcome,
    github::review_imports::claim_github_comment,
    integer_columns::{i32_to_u32, i64_to_u64, optional_i64_to_u64, u32_to_i32, u64_to_i64},
    locks::acquire_shared_repository_lock,
    repository_access::repository_access,
    request_discussion_actors::save_github_accounts,
    request_discussion_rows::insert_discussion,
    request_rows::{
        insert_request_event_row, insert_request_row, request_by_github_issue_number,
        request_by_id, request_by_name,
    },
};
use crate::error::PostgresError;
use scope_domain::{
    github::issue_import::{
        GitHubIssueImport, GitHubIssueImportProgress, GitHubIssuePosition, GitHubSkippedIssue,
        start_github_issue_import,
    },
    repository::RepositoryIncarnation,
    requests::{
        GitHubIssue, GitHubIssueComments, GitHubIssueImportResult, ImportGitHubIssueInput,
        OpenRequestFromAskFacts, github_issue_request_name, import_github_issue,
    },
    views::ViewId,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, TransactionTrait};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

const ISSUE_IMPORTS: GitHubImportQueue = GitHubImportQueue {
    table: "scope_github_issue_imports",
    columns: "repo_id, github_repository_id, view_id, state, attempts, imported_count, \
        already_imported_count, skipped_issues, read_through_created_at_unix, \
        read_through_issue_number, last_error, queued_at_unix, finished_at_unix",
    label: "GitHub issue import",
};

#[derive(FromQueryResult)]
struct ImportRow {
    repo_id: String,
    github_repository_id: i64,
    view_id: String,
    state: String,
    attempts: i32,
    imported_count: i32,
    already_imported_count: i32,
    skipped_issues: serde_json::Value,
    read_through_created_at_unix: i64,
    read_through_issue_number: i64,
    last_error: Option<String>,
    queued_at_unix: i64,
    finished_at_unix: Option<i64>,
}

#[derive(Serialize, Deserialize)]
struct SkippedIssueRow {
    number: u64,
    reason: String,
}

#[derive(Clone, Debug)]
pub struct ImportGitHubIssueCommand {
    pub repo_id: String,
    pub claim_token: String,
    pub issue: GitHubIssue,
    pub comments: GitHubIssueComments,
    pub main_oid: String,
    pub request_id: String,
    pub started_event_id: String,
    pub now_unix: u64,
    pub lease_until_unix: u64,
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "start_github_issue_import"))]
    pub async fn start_github_issue_import(
        &self,
        repo_id: &str,
        user_id: &str,
        view: ViewId,
        now_unix: u64,
    ) -> Result<(GitHubIssueImport, RepositoryIncarnation), PostgresError> {
        let (tx, context) = self
            .begin_github_change(
                repo_id,
                user_id,
                &["github-connection", "github-issue-import"],
            )
            .await?;
        let connection = repository_github_connection(&tx, repo_id).await?;
        let current = load_import(&tx, repo_id).await?;
        let import = start_github_issue_import(
            &context.access,
            &context.views,
            connection.as_ref(),
            current.as_ref(),
            view,
            now_unix,
        )?;
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_github_issue_imports (repo_id, github_repository_id, view_id,
                state, attempts, imported_count, already_imported_count,
                skipped_issues, read_through_created_at_unix,
                read_through_issue_number, last_error, next_attempt_at_unix, lease_until_unix,
                claim_token, queued_at_unix, finished_at_unix)
             VALUES ($1, $2, $3, 'queued', 0, 0, 0, '[]', 0, 0, NULL, $4, NULL, NULL, $4, NULL)
             ON CONFLICT (repo_id) DO UPDATE SET
                github_repository_id = EXCLUDED.github_repository_id,
                view_id = EXCLUDED.view_id, state = EXCLUDED.state,
                attempts = EXCLUDED.attempts, imported_count = EXCLUDED.imported_count,
                already_imported_count = EXCLUDED.already_imported_count,
                skipped_issues = EXCLUDED.skipped_issues,
                read_through_created_at_unix = EXCLUDED.read_through_created_at_unix,
                read_through_issue_number = EXCLUDED.read_through_issue_number,
                last_error = EXCLUDED.last_error,
                next_attempt_at_unix = EXCLUDED.next_attempt_at_unix,
                lease_until_unix = EXCLUDED.lease_until_unix,
                claim_token = EXCLUDED.claim_token, queued_at_unix = EXCLUDED.queued_at_unix,
                finished_at_unix = EXCLUDED.finished_at_unix",
            [
                import.repository_id.clone().into(),
                u64_to_i64(import.github_repository_id, "GitHub repository id")?.into(),
                import.view.as_str().into(),
                u64_to_i64(import.queued_at_unix, "GitHub issue import time")?.into(),
            ],
        ))
        .await?;
        tx.commit().await?;
        Ok((import, context.incarnation()))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_issue_import"))]
    pub async fn github_issue_import(
        &self,
        repo_id: &str,
    ) -> Result<Option<GitHubIssueImport>, PostgresError> {
        load_import(self.db.as_ref(), repo_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_due_github_issue_imports"))]
    pub async fn claim_due_github_issue_imports(
        &self,
        claim_token: &str,
        now_unix: u64,
        lease_until_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubIssueImport>, PostgresError> {
        ISSUE_IMPORTS
            .claim_due::<ImportRow>(
                self.db.as_ref(),
                claim_token,
                now_unix,
                lease_until_unix,
                limit,
            )
            .await?
            .into_iter()
            .map(ImportRow::into_domain)
            .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "renew_github_issue_import_lease"))]
    pub async fn renew_github_issue_import_lease(
        &self,
        repo_id: &str,
        claim_token: &str,
        lease_until_unix: u64,
    ) -> Result<bool, PostgresError> {
        ISSUE_IMPORTS
            .renew_lease(self.db.as_ref(), repo_id, claim_token, lease_until_unix)
            .await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "finish_github_issue_import"))]
    pub async fn finish_github_issue_import(
        &self,
        repo_id: &str,
        claim_token: &str,
        outcome: GitHubJobOutcome,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        ISSUE_IMPORTS
            .finish(self.db.as_ref(), repo_id, claim_token, &outcome, now_unix)
            .await
    }
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "import_github_issue"))]
    pub async fn import_github_issue(
        &self,
        command: ImportGitHubIssueCommand,
    ) -> Result<Option<GitHubIssueImportResult>, PostgresError> {
        let repo_id = command.repo_id.as_str();
        let tx = self.db.begin().await?;
        acquire_shared_repository_lock(&tx, repo_id).await?;
        acquire_aggregate_lock(&tx, "github-connection", repo_id).await?;
        acquire_aggregate_lock(&tx, "request-start", repo_id).await?;
        acquire_aggregate_lock(&tx, "request", &command.request_id).await?;
        let Some(import) = ISSUE_IMPORTS
            .lock_claimed::<ImportRow>(&tx, repo_id, &command.claim_token)
            .await?
        else {
            return Ok(None);
        };
        let import = import.into_domain()?;
        let repo = repository_access(&tx, repo_id, None)
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        let (issue_number, issue_position) = (command.issue.number, command.issue.position());
        let facts = OpenRequestFromAskFacts {
            request_id_exists: request_by_id(&tx, &command.request_id).await?.is_some(),
            request_name_exists: request_by_name(
                &tx,
                repo_id,
                &github_issue_request_name(issue_number),
            )
            .await?
            .is_some(),
            github_issue_has_request: request_by_github_issue_number(&tx, repo_id, issue_number)
                .await?
                .is_some(),
        };
        let comment_count = match &command.comments {
            GitHubIssueComments::Read { comments, .. } => comments.len(),
            GitHubIssueComments::IssueGone => 0,
        };
        let discussion_ids = (0..comment_count)
            .map(|_| generate_id(GeneratedIdKind::RequestDiscussion))
            .collect::<Result<_, _>>()?;
        let result = import_github_issue(
            facts,
            command.issue,
            command.comments,
            ImportGitHubIssueInput {
                request_id: command.request_id,
                repo_id: command.repo_id.clone(),
                view: import.view,
                main_oid: command.main_oid,
                started_event_id: command.started_event_id,
                discussion_ids,
                now_unix: command.now_unix,
            },
            &repo.views,
        )?;
        let Ok(step) = result.step() else {
            return Ok(Some(result));
        };
        if let GitHubIssueImportResult::Imported(imported) = &result {
            save_github_accounts(&tx, &imported.accounts, command.now_unix).await?;
            insert_request_row(&tx, &imported.request).await?;
            insert_request_event_row(&tx, &imported.started).await?;
            for discussion in &imported.discussions {
                claim_github_comment(&tx, &imported.request.id, &discussion.client_discussion_id)
                    .await?;
                insert_discussion(&tx, discussion).await?;
            }
        }
        let mut progress = import.progress;
        progress.record(issue_number, issue_position, step);
        save_progress(&tx, repo_id, &progress, command.lease_until_unix).await?;
        tx.commit().await?;
        Ok(Some(result))
    }
}

pub(in crate::db) async fn delete_github_issue_import<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<(), PostgresError> {
    ISSUE_IMPORTS.delete(conn, repo_id).await
}

async fn save_progress<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    progress: &GitHubIssueImportProgress,
    lease_until_unix: u64,
) -> Result<(), PostgresError> {
    let lease = u64_to_i64(lease_until_unix, "GitHub issue import lease")?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_github_issue_imports
            SET imported_count = $2, already_imported_count = $3, skipped_issues = $4,
                read_through_created_at_unix = $5, read_through_issue_number = $6,
                lease_until_unix = greatest(lease_until_unix, $7),
                next_attempt_at_unix = greatest(next_attempt_at_unix, $7)
          WHERE repo_id = $1",
        [
            repo_id.into(),
            u32_to_i32(progress.imported_count, "GitHub issue import count")?.into(),
            u32_to_i32(progress.already_imported_count, "GitHub issue import count")?.into(),
            serde_json::json!(
                progress
                    .skipped
                    .iter()
                    .map(|skipped| SkippedIssueRow {
                        number: skipped.number,
                        reason: skipped.reason.clone(),
                    })
                    .collect::<Vec<_>>()
            )
            .into(),
            u64_to_i64(
                progress.read_through.created_at_unix,
                "GitHub issue creation time",
            )?
            .into(),
            u64_to_i64(progress.read_through.number, "GitHub issue number")?.into(),
            lease.into(),
        ],
    ))
    .await?;
    Ok(())
}

async fn load_import<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<GitHubIssueImport>, PostgresError> {
    ISSUE_IMPORTS
        .load::<ImportRow>(conn, repo_id)
        .await?
        .map(ImportRow::into_domain)
        .transpose()
}

impl ImportRow {
    fn into_domain(self) -> Result<GitHubIssueImport, PostgresError> {
        Ok(GitHubIssueImport {
            repository_id: self.repo_id,
            github_repository_id: i64_to_u64(self.github_repository_id, "GitHub repository id")?,
            view: ViewId::parse(&self.view_id)?,
            state: decode_enum(self.state)?,
            attempts: i32_to_u32(self.attempts, "GitHub issue import attempts")?,
            progress: GitHubIssueImportProgress {
                imported_count: i32_to_u32(self.imported_count, "GitHub issue import count")?,
                already_imported_count: i32_to_u32(
                    self.already_imported_count,
                    "GitHub issue import count",
                )?,
                skipped: serde_json::from_value::<Vec<SkippedIssueRow>>(self.skipped_issues)
                    .map_err(PostgresError::internal)?
                    .into_iter()
                    .map(|skipped| GitHubSkippedIssue {
                        number: skipped.number,
                        reason: skipped.reason,
                    })
                    .collect(),
                read_through: GitHubIssuePosition {
                    created_at_unix: i64_to_u64(
                        self.read_through_created_at_unix,
                        "GitHub issue creation time",
                    )?,
                    number: i64_to_u64(self.read_through_issue_number, "GitHub issue number")?,
                },
            },
            last_error: self.last_error,
            queued_at_unix: i64_to_u64(self.queued_at_unix, "GitHub issue import time")?,
            finished_at_unix: optional_i64_to_u64(
                self.finished_at_unix,
                "GitHub issue import end",
            )?,
        })
    }
}
