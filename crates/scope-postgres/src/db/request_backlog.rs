use super::{
    RequestStore,
    auth::load_users_by_ids,
    begin_metadata_read_snapshot,
    request_queue::{escaped_search_pattern, request_visibility_sql, restricted_to_request_views},
};
use crate::{
    db::integer_columns::{i64_to_u64, u64_to_i64},
    error::PostgresError,
};
use scope_domain::{
    account::UserAccount,
    requests::{
        REQUEST_LIST_MAX_PAGE_SIZE, RequestBacklogSearch, RequestBacklogSort,
        RequestBacklogWorking, RequestViewer, request_backlog_predicate,
    },
    views::{ViewId, Views},
};
use sea_orm::{DatabaseBackend, FromQueryResult, Statement, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestBacklogCursor {
    pub asked_at_unix: u64,
    pub github_issue_number: u64,
}

#[derive(Clone, Debug)]
pub struct RequestBacklogPageQuery<'a> {
    pub repo_id: &'a str,
    pub viewer: &'a RequestViewer<'a>,
    pub views: &'a Views,
    pub request_views: Option<Vec<ViewId>>,
    pub search: Option<&'a RequestBacklogSearch>,
    pub working: RequestBacklogWorking,
    pub sort: RequestBacklogSort,
    pub after: Option<&'a RequestBacklogCursor>,
    pub limit: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestBacklogRow {
    pub request_id: String,
    pub title: String,
    pub github_issue_number: u64,
    pub asker_login: String,
    pub asked_at_unix: u64,
    pub worker_user_id: Option<String>,
    pub message_count: u64,
}

impl RequestBacklogRow {
    pub fn cursor(&self) -> RequestBacklogCursor {
        RequestBacklogCursor {
            asked_at_unix: self.asked_at_unix,
            github_issue_number: self.github_issue_number,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RequestBacklogPage {
    pub rows: Vec<RequestBacklogRow>,
    pub users: BTreeMap<String, UserAccount>,
    pub total: u64,
}

#[derive(Debug, FromQueryResult)]
struct BacklogModel {
    id: String,
    title: String,
    author_user_id: Option<String>,
    github_issue_number: i64,
    asked_at_unix: i64,
    asker_login: String,
    message_count: i64,
}

#[derive(Debug, FromQueryResult)]
struct BacklogTotal {
    total: i64,
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_backlog_page"))]
    pub async fn request_backlog_page(
        &self,
        input: RequestBacklogPageQuery<'_>,
    ) -> Result<RequestBacklogPage, PostgresError> {
        let matching = BACKLOG_MATCH_SQL.replace(
            "{backlog_membership}",
            &request_visibility_sql(&restricted_to_request_views(
                request_backlog_predicate(input.viewer, input.views),
                input.request_views.as_deref(),
            )),
        );
        let matched_by: [Value; 6] = [
            input.repo_id.into(),
            input.viewer.user_id.map(str::to_string).into(),
            working_name(input.working).into(),
            input.search.is_some().into(),
            input
                .search
                .and_then(|search| search.github_issue_number)
                .and_then(|number| i64::try_from(number).ok())
                .into(),
            input
                .search
                .and_then(|search| search.text.as_deref())
                .map(escaped_search_pattern)
                .into(),
        ];
        let (after, order) = match input.sort {
            RequestBacklogSort::Oldest => (">", "ASC"),
            RequestBacklogSort::Newest => ("<", "DESC"),
        };
        let page_sql = BACKLOG_PAGE_SQL
            .replace("{matching}", &matching)
            .replace("{after}", after)
            .replace("{order}", order);
        let limit = input.limit.min((REQUEST_LIST_MAX_PAGE_SIZE + 1) as u64);
        let paged_by: [Value; 3] = [
            input
                .after
                .map(|cursor| u64_to_i64(cursor.asked_at_unix, "backlog cursor time"))
                .transpose()?
                .into(),
            input
                .after
                .map(|cursor| u64_to_i64(cursor.github_issue_number, "backlog cursor issue"))
                .transpose()?
                .into(),
            i64::try_from(limit)
                .map_err(PostgresError::internal)?
                .into(),
        ];
        let snapshot = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let total = BacklogTotal::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!("SELECT count(*) AS total {matching}"),
            matched_by.clone(),
        ))
        .one(&snapshot)
        .await?
        .ok_or_else(|| PostgresError::internal_message("backlog count returned no row"))?
        .total;
        let rows = BacklogModel::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            page_sql,
            matched_by.into_iter().chain(paged_by),
        ))
        .all(&snapshot)
        .await?
        .into_iter()
        .map(BacklogModel::try_into_row)
        .collect::<Result<Vec<_>, _>>()?;
        let users = load_users_by_ids(
            &snapshot,
            rows.iter().filter_map(|row| row.worker_user_id.clone()),
        )
        .await?;
        snapshot.commit().await?;
        Ok(RequestBacklogPage {
            rows,
            users,
            total: i64_to_u64(total, "backlog total")?,
        })
    }
}

impl BacklogModel {
    fn try_into_row(self) -> Result<RequestBacklogRow, PostgresError> {
        Ok(RequestBacklogRow {
            request_id: self.id,
            title: self.title,
            github_issue_number: i64_to_u64(self.github_issue_number, "GitHub issue number")?,
            asker_login: self.asker_login,
            asked_at_unix: i64_to_u64(self.asked_at_unix, "ask time")?,
            worker_user_id: self.author_user_id,
            message_count: i64_to_u64(self.message_count, "request message count")?,
        })
    }
}

fn working_name(working: RequestBacklogWorking) -> &'static str {
    match working {
        RequestBacklogWorking::Anyone => "anyone",
        RequestBacklogWorking::Nobody => "nobody",
        RequestBacklogWorking::Me => "me",
    }
}

const BACKLOG_MATCH_SQL: &str = r#"
FROM scope_requests r
WHERE r.repo_id = $1
  AND {backlog_membership}
  AND CASE $3::text
        WHEN 'nobody' THEN r.author_user_id IS NULL
        WHEN 'me' THEN r.author_user_id = $2::text
        ELSE TRUE
      END
  AND (
    NOT $4::boolean
    OR r.github_issue_number = $5::bigint
    OR r.title ILIKE $6::text ESCAPE '\'
    OR r.ask_text_markdown ILIKE $6::text ESCAPE '\'
  )
"#;

const BACKLOG_PAGE_SQL: &str = r#"
SELECT r.id, r.title, r.author_user_id, r.github_issue_number, r.asked_at_unix,
    (SELECT asker.login FROM scope_github_accounts asker
     WHERE asker.github_user_id = r.asker_github_user_id) AS asker_login,
    (SELECT count(*) FROM scope_request_discussions d WHERE d.request_id = r.id)
    + (SELECT count(*) FROM scope_request_discussion_replies reply
       JOIN scope_request_discussions d ON d.id = reply.discussion_id
       WHERE d.request_id = r.id) AS message_count
{matching}
  AND (
    $7::bigint IS NULL
    OR (r.asked_at_unix, r.github_issue_number) {after} ($7::bigint, $8::bigint)
  )
ORDER BY r.asked_at_unix {order}, r.github_issue_number {order}
LIMIT $9
"#;

#[cfg(test)]
mod tests;
