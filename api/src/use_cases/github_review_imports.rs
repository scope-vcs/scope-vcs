use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{
    error::ApiError,
    github::{GitHubPullReview, GitHubPullReviewComment, webhook::GitHubReviewDelivery},
    persistence::unix_now,
    state::AppState,
    use_cases::{
        github_review_bot_statuses::note_review_bot_activity,
        request_discussion_mutation::first_valid_anchor,
    },
};
use authors::ImportAuthors;
use scope_domain::{
    github_account::GitHubAccount,
    github_connection::GitHubConnection,
    repository::access::{RepositoryAccess, RepositoryAccessContext},
    requests::{
        GitHubReviewImportSubject, Request, RequestDiscussion, RequestDiscussionStatus,
        RequestRevision, github_comment_anchors, github_comment_body, github_retry_at,
        github_review_anchor, github_review_client_id, github_review_comment_client_id,
        github_thread_status,
    },
};
use scope_postgres::db::{
    GitHubImportedComment, GitHubReviewImportFinish, GitHubReviewImportJob, GitHubThreadTransition,
    ImportGitHubReviewCommand, QueueGitHubReviewImport, TransitionGitHubDiscussionCommand,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

mod authors;

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const BATCH_SIZE: u64 = 10;
const LEASE_SECS: u64 = 5 * 60;

pub(crate) async fn queue_github_review_delivery(
    state: &AppState,
    delivery: GitHubReviewDelivery,
    now_unix: u64,
) -> Result<(), ApiError> {
    let Some(connection) = state
        .metadata
        .repositories()
        .github_connection_for_github_repository(delivery.github_repository_id)
        .await?
    else {
        return Ok(());
    };
    state
        .metadata
        .requests()
        .queue_github_review_import(
            &QueueGitHubReviewImport {
                request_id: delivery.request_id,
                repo_id: connection.repository_id,
                github_repository_id: delivery.github_repository_id,
                pull_number: delivery.pull_number,
                subject: delivery.subject,
            },
            &delivery.accounts,
            now_unix,
        )
        .await?;
    state.wakeups.github_review_imports.notify_one();
    Ok(())
}

pub(crate) async fn import_due_github_reviews(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let requests = state.metadata.requests();
    let jobs = requests
        .claim_due_github_review_imports(now_unix, now_unix.saturating_add(LEASE_SECS), BATCH_SIZE)
        .await?;
    let mut imported = 0;
    for job in jobs {
        let finish = match run_job(state, &job).await {
            Ok(None) => {
                imported += 1;
                GitHubReviewImportFinish::Done
            }
            Ok(Some(refusals)) => {
                imported += 1;
                GitHubReviewImportFinish::Record {
                    error: refusals,
                    retry_at_unix: None,
                }
            }
            Err(error) => {
                let retry_at_unix = github_retry_at(job.attempts, now_unix);
                let error = error.into_operator_diagnostic();
                tracing::warn!(
                    request_id = job.request_id,
                    attempts = job.attempts,
                    gives_up = retry_at_unix.is_none(),
                    %error,
                    "importing a GitHub review failed"
                );
                GitHubReviewImportFinish::Record {
                    error,
                    retry_at_unix,
                }
            }
        };
        requests.finish_github_review_import(&job, finish).await?;
    }
    Ok(imported)
}

async fn run_job(
    state: &AppState,
    job: &GitHubReviewImportJob,
) -> Result<Option<String>, ApiError> {
    let Some(connection) = state
        .metadata
        .repositories()
        .github_connection(&job.repo_id)
        .await?
        .filter(|connection| connection.is_connected_to(job.github_repository_id))
    else {
        return Ok(None);
    };
    let mut authors = ImportAuthors::load(state, &connection).await?;
    import_subject(state, &connection, job, &mut authors).await?;
    Ok(authors.refusals())
}

async fn import_subject(
    state: &AppState,
    connection: &GitHubConnection,
    job: &GitHubReviewImportJob,
    authors: &mut ImportAuthors<'_>,
) -> Result<(), ApiError> {
    match job.subject {
        GitHubReviewImportSubject::Review { review_id } => {
            let app = state.github_app()?;
            let Some(review) = app
                .pull_request_review(
                    connection.installation_id,
                    &connection.github_full_name,
                    job.pull_number,
                    review_id,
                )
                .await?
            else {
                return Ok(());
            };
            let comments = app
                .pull_request_review_comments(
                    connection.installation_id,
                    &connection.github_full_name,
                    job.pull_number,
                    review_id,
                )
                .await?
                .unwrap_or_default();
            if import_comments(state, connection, job, authors, Some(review), comments).await? {
                state
                    .metadata
                    .requests()
                    .record_github_review_imported(&job.request_id, review_id)
                    .await?;
                note_review_bot_activity(state, job.github_repository_id, job.pull_number).await?;
            }
            Ok(())
        }
        GitHubReviewImportSubject::Reply { comment_id } => {
            let Some(comment) = state
                .github_app()?
                .pull_request_review_comment(
                    connection.installation_id,
                    &connection.github_full_name,
                    comment_id,
                )
                .await?
            else {
                return Ok(());
            };
            import_comments(state, connection, job, authors, None, vec![comment])
                .await
                .map(drop)
        }
        GitHubReviewImportSubject::Thread {
            root_comment_id,
            ref node_id,
            resolved,
            github_account_id,
        } => {
            let Some(thread) = state
                .github_app()?
                .review_thread(connection.installation_id, node_id)
                .await?
            else {
                return Ok(());
            };
            if thread.root_comment_id != Some(root_comment_id) {
                return Ok(());
            }
            let Some(status) = github_thread_status(resolved, thread.resolved) else {
                return Ok(());
            };
            let actor = match status {
                RequestDiscussionStatus::Resolved => thread.resolver,
                RequestDiscussionStatus::Open => {
                    state
                        .metadata
                        .requests()
                        .github_account(github_account_id)
                        .await?
                }
            };
            let Some(actor) = actor else {
                return Ok(());
            };
            if !authors.admits(&actor).await? {
                return Ok(());
            }
            let transition = state
                .metadata
                .requests()
                .transition_github_discussion(TransitionGitHubDiscussionCommand {
                    request_id: job.request_id.clone(),
                    github_link_id: job.github_link_id.clone(),
                    root_comment_id,
                    claim_token: job.claim_token.clone(),
                    resolver: actor,
                    status,
                    now_unix: unix_now()?,
                })
                .await?;
            match transition {
                GitHubThreadTransition::Applied(import) => {
                    publish(state, &import.request, &import.discussions).await;
                    Ok(())
                }
                GitHubThreadTransition::Unchanged
                | GitHubThreadTransition::Superseded
                | GitHubThreadTransition::Ignored => Ok(()),
                GitHubThreadTransition::MissingDiscussion => Err(ApiError::conflict(
                    "the GitHub thread's root comment has not been imported yet",
                )),
            }
        }
    }
}

async fn import_comments(
    state: &AppState,
    connection: &GitHubConnection,
    job: &GitHubReviewImportJob,
    authors: &mut ImportAuthors<'_>,
    review: Option<GitHubPullReview>,
    fetched: Vec<GitHubPullReviewComment>,
) -> Result<bool, ApiError> {
    let requests = state.metadata.requests();
    let Some(request) = requests
        .request_by_id(&job.request_id)
        .await?
        .filter(|request| request.repo_id == job.repo_id)
    else {
        return Ok(false);
    };
    let review = match review {
        Some(review) if authors.admits(&review.author).await? => Some(review),
        _ => None,
    };
    let mut comments = Vec::with_capacity(fetched.len());
    for comment in fetched {
        if authors.admits(&comment.author).await? {
            comments.push(comment);
        }
    }
    let known = comments
        .iter()
        .map(|comment| comment.id)
        .collect::<BTreeSet<_>>();
    let missing_roots = comments
        .iter()
        .filter_map(|comment| comment.in_reply_to_id)
        .filter(|root_id| !known.contains(root_id))
        .collect::<BTreeSet<_>>();
    for root_id in missing_roots {
        if requests
            .request_has_github_discussion(&request.id, &github_review_comment_client_id(root_id))
            .await?
        {
            continue;
        }
        if let Some(root) = state
            .github_app()?
            .pull_request_review_comment(
                connection.installation_id,
                &connection.github_full_name,
                root_id,
            )
            .await?
            && authors.admits(&root.author).await?
        {
            comments.push(root);
        }
    }
    comments.sort_by_key(|comment| (comment.in_reply_to_id.is_some(), comment.id));
    let revisions = revisions_for_commits(
        state,
        &request,
        review
            .iter()
            .filter_map(|review| review.commit_oid.as_deref())
            .chain(comments.iter().flat_map(|comment| {
                [
                    comment.position.commit_oid.as_str(),
                    comment.position.original_commit_oid.as_str(),
                ]
            })),
    )
    .await?;
    let repo = full_view_context(state, &request.repo_id).await?;
    let now_unix = unix_now()?;
    let mut accounts = BTreeMap::<u64, GitHubAccount>::new();
    let mut imported = Vec::new();
    if let Some(review) = review
        && let Some(body) = github_comment_body(&review.body)
    {
        accounts.insert(review.author.id, review.author.clone());
        imported.push(GitHubImportedComment {
            client_id: github_review_client_id(review.id),
            reply_to_client_id: None,
            github_account_id: review.author.id,
            source_url: review.source_url,
            body_markdown: body,
            anchor: github_review_anchor(review.commit_oid.as_deref(), &revisions),
            created_at_unix: review.submitted_at_unix.unwrap_or(now_unix),
        });
    }
    for comment in comments {
        let Some(body) = github_comment_body(&comment.body) else {
            continue;
        };
        let anchor = match comment.in_reply_to_id {
            Some(root_id) if known.contains(&root_id) => None,
            _ => {
                first_valid_anchor(
                    state,
                    &repo,
                    &request,
                    github_comment_anchors(&comment.position, &revisions),
                )
                .await?
            }
        };
        accounts.insert(comment.author.id, comment.author.clone());
        imported.push(GitHubImportedComment {
            client_id: github_review_comment_client_id(comment.id),
            reply_to_client_id: comment.in_reply_to_id.map(github_review_comment_client_id),
            github_account_id: comment.author.id,
            source_url: comment.source_url,
            body_markdown: body,
            anchor,
            created_at_unix: comment.created_at_unix.unwrap_or(now_unix),
        });
    }
    if imported.is_empty() {
        return Ok(true);
    }
    let Some(import) = requests
        .import_github_review(ImportGitHubReviewCommand {
            request_id: request.id.clone(),
            github_link_id: job.github_link_id.clone(),
            accounts: accounts.into_values().collect(),
            comments: imported,
            now_unix,
        })
        .await?
    else {
        return Ok(false);
    };
    publish(state, &import.request, &import.discussions).await;
    Ok(true)
}

async fn revisions_for_commits<'a>(
    state: &AppState,
    request: &Request,
    commits: impl Iterator<Item = &'a str>,
) -> Result<Vec<RequestRevision>, ApiError> {
    let mut revisions = Vec::new();
    for commit_oid in commits.collect::<BTreeSet<_>>() {
        if let Some(revision) = state
            .metadata
            .requests()
            .request_revision_with_head(&request.id, commit_oid)
            .await?
        {
            revisions.push(revision);
        }
    }
    Ok(revisions)
}

async fn full_view_context(
    state: &AppState,
    repo_id: &str,
) -> Result<RepositoryAccessContext, ApiError> {
    let repositories = state.metadata.repositories();
    let record = repositories
        .repository_record(repo_id)
        .await?
        .ok_or_else(|| ApiError::not_found("repo not found"))?;
    let views = repositories
        .repository_views(&record.incarnation(), record.content_version)
        .await?;
    Ok(RepositoryAccessContext {
        record,
        access: RepositoryAccess::full_view_reader(&views),
        views,
    })
}

async fn publish(state: &AppState, request: &Request, discussions: &[RequestDiscussion]) {
    let incarnation = match state.repository_incarnation(&request.repo_id).await {
        Ok(Some(incarnation)) => incarnation,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(
                request_id = request.id,
                error = %error.operator_diagnostic(),
                "could not announce imported GitHub review comments"
            );
            return;
        }
    };
    for discussion in discussions {
        state
            .publish_request_timeline_change(
                &incarnation,
                request.id.clone(),
                discussion.id.clone(),
                discussion.last_activity_position,
                request.view.clone(),
            )
            .await;
    }
}

impl BackgroundWork {
    pub(crate) fn start_github_review_imports(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "GitHub review import pass",
                cadence: Cadence::AfterPass {
                    delay: POLL_INTERVAL,
                    wakeup: Some(|wakeups| &wakeups.github_review_imports),
                },
                readiness: Readiness::SchemaRequired,
            },
            |state| async move { import_due_github_reviews(&state, unix_now()?).await },
        );
    }
}
