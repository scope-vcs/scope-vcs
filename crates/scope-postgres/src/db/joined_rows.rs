use crate::error::PostgresError;
use sea_orm::{IdenStatic, Iterable, QueryResult};

pub(super) fn joined(row: &QueryResult, column: &str) -> Result<bool, PostgresError> {
    Ok(row.try_get::<Option<String>>("", column)?.is_some())
}

pub(super) fn prefixed_columns<C: Iterable + IdenStatic>(table: &str, prefix: &str) -> String {
    C::iter()
        .map(|column| format!("{table}.\"{0}\" AS \"{prefix}{0}\"", column.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}
