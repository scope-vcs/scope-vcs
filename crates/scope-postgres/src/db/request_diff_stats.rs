use super::{
    RequestStore, entities,
    integer_columns::{i64_to_u64, optional_i64_to_u64, optional_u64_to_i64, u64_to_i64},
    request_revision_rows::current_revision_ids,
};
use crate::error::PostgresError;
use scope_domain::{
    policy::ScopePath,
    requests::{DiffStat, FileDiffStat, FileLineChanges, Request, RevisionDiffStat},
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, TransactionTrait, Value,
};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

const FILE_INSERT_BATCH: usize = 1_000;

#[derive(FromQueryResult)]
struct DiffStatRow {
    revision_id: String,
    additions: i64,
    deletions: i64,
    changed_files: i64,
    files_listed: bool,
}

#[derive(FromQueryResult)]
struct DiffFileRow {
    revision_id: String,
    path: String,
    additions: Option<i64>,
    deletions: Option<i64>,
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "record_request_revision_diff_stat"))]
    pub async fn record_request_revision_diff_stat(
        &self,
        revision_id: &str,
        stat: &RevisionDiffStat,
    ) -> Result<bool, PostgresError> {
        let tx = self.db.begin().await?;
        let totals = stat.totals();
        let inserted = tx
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_request_revision_diff_stats
                     (revision_id, additions, deletions, changed_files, files_listed)
                 SELECT id, $2, $3, $4, $5 FROM scope_request_revisions WHERE id = $1
                 ON CONFLICT (revision_id) DO NOTHING",
                [
                    revision_id.into(),
                    u64_to_i64(totals.additions, "diff additions")?.into(),
                    u64_to_i64(totals.deletions, "diff deletions")?.into(),
                    u64_to_i64(totals.changed_files, "diff changed files")?.into(),
                    stat.files().is_some().into(),
                ],
            ))
            .await?
            .rows_affected()
            == 1;
        if inserted {
            for files in stat.files().unwrap_or_default().chunks(FILE_INSERT_BATCH) {
                insert_files(&tx, revision_id, files).await?;
            }
        }
        tx.commit().await?;
        Ok(inserted)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "current_request_diff_totals"))]
    pub async fn current_request_diff_totals(
        &self,
        request_ids: &[String],
    ) -> Result<BTreeMap<String, DiffStat>, PostgresError> {
        self.current_diff_stat_rows(request_ids)
            .await?
            .into_iter()
            .map(|(request_id, row)| Ok((request_id, row_totals(&row)?)))
            .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "current_request_diff_stats"))]
    pub async fn current_request_diff_stats(
        &self,
        request_ids: &[String],
    ) -> Result<BTreeMap<String, RevisionDiffStat>, PostgresError> {
        let rows = self.current_diff_stat_rows(request_ids).await?;
        let listed = rows
            .values()
            .filter(|row| row.files_listed)
            .map(|row| row.revision_id.clone())
            .collect::<Vec<_>>();
        let mut files = BTreeMap::<String, Vec<FileDiffStat>>::new();
        if !listed.is_empty() {
            for row in DiffFileRow::find_by_statement(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                format!(
                    "SELECT revision_id, path, additions, deletions
                     FROM scope_request_revision_diff_files WHERE revision_id IN ({})",
                    placeholders(listed.len())
                ),
                listed.into_iter().map(Value::from),
            ))
            .all(self.db.as_ref())
            .await?
            {
                files
                    .entry(row.revision_id.clone())
                    .or_default()
                    .push(file_from_row(row)?);
            }
        }
        rows.into_iter()
            .map(|(request_id, row)| {
                let listed_files = row
                    .files_listed
                    .then(|| files.remove(&row.revision_id).unwrap_or_default());
                Ok((
                    request_id,
                    RevisionDiffStat::recorded(row_totals(&row)?, listed_files),
                ))
            })
            .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "requests_missing_current_diff_stat"))]
    pub async fn requests_missing_current_diff_stat(
        &self,
        after_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<Request>, PostgresError> {
        entities::request::Model::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT request.* FROM scope_requests request
             CROSS JOIN LATERAL (
                 SELECT revision.id FROM scope_request_revisions revision
                 WHERE revision.request_id = request.id
                 ORDER BY revision.position DESC, revision.id DESC LIMIT 1
             ) current
             WHERE ($1::text IS NULL OR request.id > $1)
               AND NOT EXISTS (
                   SELECT 1 FROM scope_request_revision_diff_stats stat
                   WHERE stat.revision_id = current.id
               )
             ORDER BY request.id LIMIT $2",
            [
                after_id.map(str::to_string).into(),
                u64_to_i64(limit, "diff stat backfill page size")?.into(),
            ],
        ))
        .all(self.db.as_ref())
        .await?
        .into_iter()
        .map(entities::request::Model::try_into_domain)
        .collect()
    }

    async fn current_diff_stat_rows(
        &self,
        request_ids: &[String],
    ) -> Result<BTreeMap<String, DiffStatRow>, PostgresError> {
        let current = current_revision_ids(self.db.as_ref(), request_ids).await?;
        if current.is_empty() {
            return Ok(BTreeMap::new());
        }
        let mut requests_by_revision = current
            .into_iter()
            .map(|(request_id, revision_id)| (revision_id, request_id))
            .collect::<BTreeMap<_, _>>();
        let revision_ids = requests_by_revision.keys().cloned().collect::<Vec<_>>();
        Ok(
            DiffStatRow::find_by_statement(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                format!(
                    "SELECT revision_id, additions, deletions, changed_files, files_listed
                 FROM scope_request_revision_diff_stats WHERE revision_id IN ({})",
                    placeholders(revision_ids.len())
                ),
                revision_ids.into_iter().map(Value::from),
            ))
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .filter_map(|row| Some((requests_by_revision.remove(&row.revision_id)?, row)))
            .collect(),
        )
    }
}

async fn insert_files<C: ConnectionTrait>(
    conn: &C,
    revision_id: &str,
    files: &[FileDiffStat],
) -> Result<(), PostgresError> {
    let mut values = Vec::with_capacity(files.len() * 4);
    let mut rows = Vec::with_capacity(files.len());
    for (index, file) in files.iter().enumerate() {
        let (additions, deletions) = match file.lines {
            FileLineChanges::Text {
                additions,
                deletions,
            } => (Some(additions), Some(deletions)),
            FileLineChanges::Binary => (None, None),
        };
        let first = index * 4 + 1;
        rows.push(format!(
            "(${first}, ${}, ${}, ${})",
            first + 1,
            first + 2,
            first + 3
        ));
        values.extend([
            Value::from(revision_id),
            Value::from(file.path.as_str()),
            optional_u64_to_i64(additions, "diff file additions")?.into(),
            optional_u64_to_i64(deletions, "diff file deletions")?.into(),
        ]);
    }
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "INSERT INTO scope_request_revision_diff_files (revision_id, path, additions, deletions) VALUES {}",
            rows.join(", ")
        ),
        values,
    ))
    .await?;
    Ok(())
}

fn row_totals(row: &DiffStatRow) -> Result<DiffStat, PostgresError> {
    Ok(DiffStat {
        additions: i64_to_u64(row.additions, "diff additions")?,
        deletions: i64_to_u64(row.deletions, "diff deletions")?,
        changed_files: i64_to_u64(row.changed_files, "diff changed files")?,
    })
}

fn file_from_row(row: DiffFileRow) -> Result<FileDiffStat, PostgresError> {
    let lines = match (
        optional_i64_to_u64(row.additions, "diff file additions")?,
        optional_i64_to_u64(row.deletions, "diff file deletions")?,
    ) {
        (Some(additions), Some(deletions)) => FileLineChanges::Text {
            additions,
            deletions,
        },
        (None, None) => FileLineChanges::Binary,
        _ => {
            return Err(PostgresError::internal_message(
                "diff file row has partial line counts",
            ));
        }
    };
    Ok(FileDiffStat {
        path: ScopePath::parse(row.path).map_err(PostgresError::internal)?,
        lines,
    })
}

fn placeholders(count: usize) -> String {
    (1..=count)
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(",")
}
