//! PostgreSQL has no unsigned integer columns, so every unsigned domain value
//! crosses a signed column boundary. This module owns that rule: one error
//! message per direction, labelled with the field being converted.

use crate::error::PostgresError;

pub(crate) fn u64_to_i64(value: u64, field: &str) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(|_| {
        PostgresError::internal_message(format!("{field} exceeds PostgreSQL bigint range"))
    })
}

pub(crate) fn i64_to_u64(value: i64, field: &str) -> Result<u64, PostgresError> {
    u64::try_from(value)
        .map_err(|_| PostgresError::internal_message(format!("{field} cannot be negative")))
}

pub(crate) fn usize_to_i64(value: usize, field: &str) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(|_| {
        PostgresError::internal_message(format!("{field} exceeds PostgreSQL bigint range"))
    })
}

pub(crate) fn u32_to_i32(value: u32, field: &str) -> Result<i32, PostgresError> {
    i32::try_from(value).map_err(|_| {
        PostgresError::internal_message(format!("{field} exceeds PostgreSQL integer range"))
    })
}

pub(crate) fn i32_to_u32(value: i32, field: &str) -> Result<u32, PostgresError> {
    u32::try_from(value)
        .map_err(|_| PostgresError::internal_message(format!("{field} cannot be negative")))
}

pub(crate) fn optional_u64_to_i64(
    value: Option<u64>,
    field: &str,
) -> Result<Option<i64>, PostgresError> {
    value.map(|value| u64_to_i64(value, field)).transpose()
}

pub(crate) fn optional_i64_to_u64(
    value: Option<i64>,
    field: &str,
) -> Result<Option<u64>, PostgresError> {
    value.map(|value| i64_to_u64(value, field)).transpose()
}

pub(crate) fn optional_u32_to_i32(
    value: Option<u32>,
    field: &str,
) -> Result<Option<i32>, PostgresError> {
    value.map(|value| u32_to_i32(value, field)).transpose()
}

pub(crate) fn optional_i32_to_u32(
    value: Option<i32>,
    field: &str,
) -> Result<Option<u32>, PostgresError> {
    value.map(|value| i32_to_u32(value, field)).transpose()
}
