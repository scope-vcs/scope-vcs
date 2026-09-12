use super::{database_u64, database_value};
use crate::{
    db::{
        RepositoryStore, begin_metadata_read_snapshot, decode_json,
        repository_access::repository_access,
    },
    error::PostgresError,
};
use scope_domain::{
    dependency_analysis::{DependencyCheck, DependencyCheckStatus, DependencyReport},
    repository::repo_id,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

impl RepositoryStore {
    pub async fn dependency_check(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
    ) -> Result<Option<DependencyCheck>, PostgresError> {
        let repo_id = repo_id(owner, name);
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let Some(access) = repository_access(&tx, &repo_id, Some(user_id)).await? else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        if !access.access.is_maintainer() {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        }
        let Some(current) = tx
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT repository.incarnation_id, repository.change_version, head.head_oid
                 FROM scope_repositories repository
                 LEFT JOIN scope_git_heads head ON head.repo_id = repository.id
                 WHERE repository.id = $1",
                [repo_id.clone().into()],
            ))
            .await
            .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        let current_incarnation = database_value::<String>(&current, "incarnation_id")?;
        let current_version =
            database_u64(&current, "change_version", "repository change version")?;
        let current_head = database_value::<Option<String>>(&current, "head_oid")?;
        let report_row = tx
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT incarnation_id, repo_version, head_oid, report
                 FROM scope_dependency_reports WHERE repo_id = $1",
                [repo_id.clone().into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        let report = report_row
            .as_ref()
            .map(|row| decode_json(database_value::<serde_json::Value>(row, "report")?))
            .transpose()?;
        let report_is_current = match &report_row {
            Some(row) => {
                database_value::<String>(row, "incarnation_id")? == current_incarnation
                    && database_u64(row, "repo_version", "dependency report repository version")?
                        == current_version
                    && Some(database_value::<String>(row, "head_oid")?) == current_head
            }
            None => false,
        };
        let job = tx
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT last_error FROM scope_dependency_analysis_jobs WHERE repo_id = $1",
                [repo_id.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        let error = job
            .as_ref()
            .map(|row| database_value::<Option<String>>(row, "last_error"))
            .transpose()?
            .flatten();
        let status = if error.is_some() {
            DependencyCheckStatus::Failed
        } else if job.is_some() || !report_is_current {
            if report.is_some() {
                DependencyCheckStatus::Updating
            } else {
                DependencyCheckStatus::Pending
            }
        } else if report.as_ref().is_some_and(report_is_unsupported) {
            DependencyCheckStatus::Unsupported
        } else {
            DependencyCheckStatus::Ready
        };
        let check = DependencyCheck {
            status,
            report,
            error,
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(check))
    }
}

fn report_is_unsupported(report: &DependencyReport) -> bool {
    report.analyzed_file_count == 0
        && !report.unsupported_files.is_empty()
        && report.gaps.is_empty()
}
