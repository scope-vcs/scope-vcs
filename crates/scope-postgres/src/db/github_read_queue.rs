use super::integer_columns::u64_to_i64;
use crate::error::PostgresError;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, Value};

pub(super) struct GitHubReadQueue {
    pub table: &'static str,
    pub key: &'static [&'static str],
    pub joined: &'static str,
    pub joined_on: &'static str,
    pub returning: &'static str,
    pub label: &'static str,
}

impl GitHubReadQueue {
    pub(super) async fn claim_due<R: FromQueryResult>(
        &self,
        conn: &impl ConnectionTrait,
        now_unix: u64,
        retry_at_unix: u64,
        limit: u64,
    ) -> Result<Vec<R>, PostgresError> {
        let Self {
            table,
            key,
            joined,
            joined_on,
            returning,
            label,
        } = self;
        let due_key = key.join(", ");
        let matches_due = key
            .iter()
            .map(|column| format!("read.{column} = due.{column}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let time = format!("{label} time");
        Ok(R::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "UPDATE {table} read
                    SET attempts = read.attempts + 1, next_attempt_at_unix = $2
                   FROM (SELECT {due_key}
                           FROM {table}
                          WHERE next_attempt_at_unix <= $1
                          ORDER BY next_attempt_at_unix
                          LIMIT $3
                          FOR UPDATE SKIP LOCKED) due{joined}
                  WHERE {matches_due}{joined_on}
              RETURNING {returning}"
            ),
            [
                u64_to_i64(now_unix, &time)?.into(),
                u64_to_i64(retry_at_unix, &time)?.into(),
                u64_to_i64(limit, &format!("{label} batch size"))?.into(),
            ],
        ))
        .all(conn)
        .await?)
    }

    pub(super) async fn finish(
        &self,
        conn: &impl ConnectionTrait,
        claim_columns: &[&str],
        mut values: Vec<Value>,
        retry_at_unix: Option<u64>,
    ) -> Result<(), PostgresError> {
        let table = self.table;
        let claimed = self
            .key
            .iter()
            .chain(claim_columns)
            .enumerate()
            .map(|(index, column)| format!("{column} = ${}", index + 1))
            .collect::<Vec<_>>()
            .join(" AND ");
        let sql = match retry_at_unix {
            None => format!("DELETE FROM {table} WHERE {claimed}"),
            Some(retry_at) => {
                values.push(u64_to_i64(retry_at, &format!("{} time", self.label))?.into());
                format!(
                    "UPDATE {table} SET next_attempt_at_unix = ${} WHERE {claimed}",
                    values.len()
                )
            }
        };
        conn.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .await?;
        Ok(())
    }
}
