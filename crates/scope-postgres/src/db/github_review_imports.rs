use super::{
    RequestStore, acquire_aggregate_lock,
    entities::{decode_enum, encode_enum},
    generated_ids::{GeneratedIdKind, generate_id},
    github_read_queue::GitHubReadQueue,
    integer_columns::{
        i32_to_u32, i64_to_u64, optional_i64_to_u64, optional_u64_to_i64, u64_to_i64,
    },
    locks::acquire_shared_repository_lock,
    projection_read_models::repository_views,
    request_attention::reactivate_author_attention_for_github_activity,
    request_discussion_actors::save_github_accounts,
    request_discussion_rows::{
        github_discussion_by_client_id, github_reply_by_client_id, insert_discussion, insert_reply,
        save_discussion,
    },
    request_rows::{request_by_id, save_request_row},
};
use crate::error::PostgresError;
use scope_domain::{
    github_account::GitHubAccount,
    requests::{
        GitHubReviewImportSubject, ImportGitHubDiscussionInput, ImportGitHubReplyInput, Request,
        RequestDiscussion, RequestDiscussionAnchor, RequestDiscussionStatus,
        accepts_github_reviews, import_github_discussion, import_github_reply,
        transition_github_discussion,
    },
    views::Views,
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseTransaction, FromQueryResult, Statement,
    TransactionTrait,
};

#[cfg(test)]
mod tests;

const REVIEW: &str = "review";
const REPLY: &str = "reply";
const THREAD: &str = "thread";

const REVIEW_IMPORTS: GitHubReadQueue = GitHubReadQueue {
    table: "scope_github_review_imports",
    key: &["request_id", "subject", "github_subject_id"],
    joined: "",
    joined_on: "",
    returning: "read.request_id, read.subject, read.github_subject_id, read.repo_id, \
        read.github_repository_id, read.pull_number, read.thread_status, read.github_actor_id, \
        read.attempts",
    label: "GitHub review import",
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubReviewImportJob {
    pub request_id: String,
    pub repo_id: String,
    pub github_repository_id: u64,
    pub pull_number: u64,
    pub subject: GitHubReviewImportSubject,
    pub attempts: u32,
}

#[derive(FromQueryResult)]
struct ImportJobRow {
    request_id: String,
    subject: String,
    github_subject_id: i64,
    repo_id: String,
    github_repository_id: i64,
    pull_number: i64,
    thread_status: Option<String>,
    github_actor_id: Option<i64>,
    attempts: i32,
}

struct SubjectColumns {
    subject: &'static str,
    github_subject_id: u64,
    thread_status: Option<String>,
    github_actor_id: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct GitHubImportedComment {
    pub client_id: String,
    pub reply_to_client_id: Option<String>,
    pub github_account_id: u64,
    pub source_url: String,
    pub body_markdown: String,
    pub anchor: Option<RequestDiscussionAnchor>,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug)]
pub struct ImportGitHubReviewCommand {
    pub request_id: String,
    pub accounts: Vec<GitHubAccount>,
    pub comments: Vec<GitHubImportedComment>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubReviewImport {
    pub request: Request,
    pub discussions: Vec<RequestDiscussion>,
}

#[derive(Clone, Debug)]
pub struct TransitionGitHubDiscussionCommand {
    pub request_id: String,
    pub client_discussion_id: String,
    pub github_account_id: u64,
    pub status: RequestDiscussionStatus,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubThreadTransition {
    Applied(Box<GitHubReviewImport>),
    Unchanged,
    MissingDiscussion,
    Ignored,
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "queue_github_review_import"))]
    pub async fn queue_github_review_import(
        &self,
        job: &GitHubReviewImportJob,
        accounts: &[GitHubAccount],
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let columns = subject_columns(&job.subject)?;
        let tx = self.db.begin().await?;
        save_github_accounts(&tx, accounts, now_unix).await?;
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_github_review_imports (request_id, subject, github_subject_id,
                repo_id, github_repository_id, pull_number, thread_status, github_actor_id,
                attempts, next_attempt_at_unix)
             SELECT request.id, $2, $3, request.repo_id, $5, $6, $7, $8, 0, $9
               FROM scope_requests request
              WHERE request.id = $1 AND request.repo_id = $4
             ON CONFLICT (request_id, subject, github_subject_id) DO UPDATE
                SET thread_status = EXCLUDED.thread_status,
                    github_actor_id = EXCLUDED.github_actor_id,
                    attempts = 0,
                    next_attempt_at_unix = EXCLUDED.next_attempt_at_unix
              WHERE (scope_github_review_imports.thread_status,
                     scope_github_review_imports.github_actor_id)
                    IS DISTINCT FROM (EXCLUDED.thread_status, EXCLUDED.github_actor_id)",
            [
                job.request_id.clone().into(),
                columns.subject.into(),
                u64_to_i64(columns.github_subject_id, "GitHub review import subject")?.into(),
                job.repo_id.clone().into(),
                u64_to_i64(job.github_repository_id, "GitHub repository id")?.into(),
                u64_to_i64(job.pull_number, "GitHub pull request number")?.into(),
                columns.thread_status.into(),
                optional_u64_to_i64(columns.github_actor_id, "GitHub account id")?.into(),
                u64_to_i64(now_unix, "GitHub review import time")?.into(),
            ],
        ))
        .await?;
        tx.commit().await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_due_github_review_imports"))]
    pub async fn claim_due_github_review_imports(
        &self,
        now_unix: u64,
        retry_at_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubReviewImportJob>, PostgresError> {
        REVIEW_IMPORTS
            .claim_due::<ImportJobRow>(self.db.as_ref(), now_unix, retry_at_unix, limit)
            .await?
            .into_iter()
            .map(ImportJobRow::into_job)
            .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "finish_github_review_import"))]
    pub async fn finish_github_review_import(
        &self,
        job: &GitHubReviewImportJob,
        retry_at_unix: Option<u64>,
    ) -> Result<(), PostgresError> {
        let columns = subject_columns(&job.subject)?;
        let mut values = vec![
            job.request_id.clone().into(),
            columns.subject.into(),
            u64_to_i64(columns.github_subject_id, "GitHub review import subject")?.into(),
            columns.thread_status.into(),
            optional_u64_to_i64(columns.github_actor_id, "GitHub account id")?.into(),
        ];
        let claimed = "request_id = $1 AND subject = $2 AND github_subject_id = $3
            AND thread_status IS NOT DISTINCT FROM $4
            AND github_actor_id IS NOT DISTINCT FROM $5";
        let sql = match retry_at_unix {
            None => format!("DELETE FROM scope_github_review_imports WHERE {claimed}"),
            Some(retry_at_unix) => {
                values.push(u64_to_i64(retry_at_unix, "GitHub review import time")?.into());
                format!(
                    "UPDATE scope_github_review_imports SET next_attempt_at_unix = $6
                      WHERE {claimed}"
                )
            }
        };
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                sql,
                values,
            ))
            .await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_has_github_discussion"))]
    pub async fn request_has_github_discussion(
        &self,
        request_id: &str,
        client_discussion_id: &str,
    ) -> Result<bool, PostgresError> {
        Ok(
            github_discussion_by_client_id(self.db.as_ref(), request_id, client_discussion_id)
                .await?
                .is_some(),
        )
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "import_github_review"))]
    pub async fn import_github_review(
        &self,
        command: ImportGitHubReviewCommand,
    ) -> Result<Option<GitHubReviewImport>, PostgresError> {
        let tx = self.db.begin().await?;
        let Some((mut request, views)) = lock_importing_request(&tx, &command.request_id).await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        save_github_accounts(&tx, &command.accounts, command.now_unix).await?;
        let mut touched = Vec::<RequestDiscussion>::new();
        for comment in command.comments {
            if import_reply(
                &tx,
                &mut request,
                &views,
                &mut touched,
                &comment,
                command.now_unix,
            )
            .await?
            {
                continue;
            }
            if github_discussion_by_client_id(&tx, &request.id, &comment.client_id)
                .await?
                .is_some()
            {
                continue;
            }
            let discussion = import_github_discussion(
                &mut request,
                &views,
                ImportGitHubDiscussionInput {
                    id: generate_id(GeneratedIdKind::RequestDiscussion)?,
                    client_discussion_id: comment.client_id,
                    github_account_id: comment.github_account_id,
                    source_url: comment.source_url,
                    body_markdown: comment.body_markdown,
                    anchor: comment.anchor,
                    created_at_unix: comment.created_at_unix,
                    now_unix: command.now_unix,
                },
            )?;
            insert_discussion(&tx, &discussion).await?;
            touched.push(discussion);
        }
        if !touched.is_empty() {
            for discussion in &touched {
                save_discussion(&tx, discussion).await?;
            }
            save_request_row(&tx, &request).await?;
            reactivate_author_attention_for_github_activity(&tx, &request, command.now_unix)
                .await?;
        }
        tx.commit().await?;
        Ok(Some(GitHubReviewImport {
            request,
            discussions: touched,
        }))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "transition_github_discussion"))]
    pub async fn transition_github_discussion(
        &self,
        command: TransitionGitHubDiscussionCommand,
    ) -> Result<GitHubThreadTransition, PostgresError> {
        let tx = self.db.begin().await?;
        let Some((mut request, views)) = lock_importing_request(&tx, &command.request_id).await?
        else {
            tx.commit().await?;
            return Ok(GitHubThreadTransition::Ignored);
        };
        let Some(mut discussion) =
            github_discussion_by_client_id(&tx, &request.id, &command.client_discussion_id).await?
        else {
            tx.commit().await?;
            return Ok(GitHubThreadTransition::MissingDiscussion);
        };
        if !transition_github_discussion(
            &mut request,
            &views,
            &mut discussion,
            command.github_account_id,
            command.status,
            command.now_unix,
        )? {
            tx.commit().await?;
            return Ok(GitHubThreadTransition::Unchanged);
        }
        save_request_row(&tx, &request).await?;
        save_discussion(&tx, &discussion).await?;
        tx.commit().await?;
        Ok(GitHubThreadTransition::Applied(Box::new(
            GitHubReviewImport {
                request,
                discussions: vec![discussion],
            },
        )))
    }
}

async fn import_reply(
    tx: &DatabaseTransaction,
    request: &mut Request,
    views: &Views,
    touched: &mut Vec<RequestDiscussion>,
    comment: &GitHubImportedComment,
    now_unix: u64,
) -> Result<bool, PostgresError> {
    let Some(root_client_id) = comment.reply_to_client_id.as_deref() else {
        return Ok(false);
    };
    let loaded = match touched
        .iter()
        .position(|discussion| discussion.client_discussion_id == root_client_id)
    {
        Some(index) => Err(index),
        None => match github_discussion_by_client_id(tx, &request.id, root_client_id).await? {
            Some(root) => Ok(root),
            None => return Ok(false),
        },
    };
    let root_id = match &loaded {
        Ok(root) => &root.id,
        Err(index) => &touched[*index].id,
    };
    if github_reply_by_client_id(tx, root_id, &comment.client_id)
        .await?
        .is_some()
    {
        return Ok(true);
    }
    let index = match loaded {
        Ok(root) => {
            touched.push(root);
            touched.len() - 1
        }
        Err(index) => index,
    };
    let root = &mut touched[index];
    let reply = import_github_reply(
        request,
        views,
        root,
        ImportGitHubReplyInput {
            id: generate_id(GeneratedIdKind::RequestDiscussionReply)?,
            client_reply_id: comment.client_id.clone(),
            github_account_id: comment.github_account_id,
            source_url: comment.source_url.clone(),
            body_markdown: comment.body_markdown.clone(),
            created_at_unix: comment.created_at_unix,
            now_unix,
        },
    )?;
    insert_reply(tx, &reply).await?;
    Ok(true)
}

async fn lock_importing_request<C: ConnectionTrait>(
    conn: &C,
    request_id: &str,
) -> Result<Option<(Request, Views)>, PostgresError> {
    let Some(observed) = request_by_id(conn, request_id).await? else {
        return Ok(None);
    };
    acquire_shared_repository_lock(conn, &observed.repo_id).await?;
    acquire_aggregate_lock(conn, "request", request_id).await?;
    let Some(request) = request_by_id(conn, request_id)
        .await?
        .filter(|request| request.repo_id == observed.repo_id)
    else {
        return Ok(None);
    };
    let views = repository_views(conn, &request.repo_id).await?;
    Ok(accepts_github_reviews(&request, &views).then_some((request, views)))
}

fn subject_columns(subject: &GitHubReviewImportSubject) -> Result<SubjectColumns, PostgresError> {
    Ok(match subject {
        GitHubReviewImportSubject::Review { review_id } => SubjectColumns {
            subject: REVIEW,
            github_subject_id: *review_id,
            thread_status: None,
            github_actor_id: None,
        },
        GitHubReviewImportSubject::Reply { comment_id } => SubjectColumns {
            subject: REPLY,
            github_subject_id: *comment_id,
            thread_status: None,
            github_actor_id: None,
        },
        GitHubReviewImportSubject::Thread {
            root_comment_id,
            status,
            github_account_id,
        } => SubjectColumns {
            subject: THREAD,
            github_subject_id: *root_comment_id,
            thread_status: Some(encode_enum(*status)?),
            github_actor_id: Some(*github_account_id),
        },
    })
}

impl ImportJobRow {
    fn into_job(self) -> Result<GitHubReviewImportJob, PostgresError> {
        let github_subject_id = i64_to_u64(self.github_subject_id, "GitHub review import subject")?;
        let subject = match (
            self.subject.as_str(),
            self.thread_status,
            optional_i64_to_u64(self.github_actor_id, "GitHub account id")?,
        ) {
            (REVIEW, None, None) => GitHubReviewImportSubject::Review {
                review_id: github_subject_id,
            },
            (REPLY, None, None) => GitHubReviewImportSubject::Reply {
                comment_id: github_subject_id,
            },
            (THREAD, Some(status), Some(github_account_id)) => GitHubReviewImportSubject::Thread {
                root_comment_id: github_subject_id,
                status: decode_enum(status)?,
                github_account_id,
            },
            (subject, _, _) => {
                return Err(PostgresError::internal_message(format!(
                    "GitHub review import has an unreadable subject {subject}"
                )));
            }
        };
        Ok(GitHubReviewImportJob {
            request_id: self.request_id,
            repo_id: self.repo_id,
            github_repository_id: i64_to_u64(self.github_repository_id, "GitHub repository id")?,
            pull_number: i64_to_u64(self.pull_number, "GitHub pull request number")?,
            subject,
            attempts: i32_to_u32(self.attempts, "GitHub review import attempts")?,
        })
    }
}
