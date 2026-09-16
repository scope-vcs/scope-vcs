//! JSON column encoding shared by entity mappers.

use crate::error::PostgresError;
use serde::{Serialize, de::DeserializeOwned};

pub(super) fn encode_json<T: Serialize>(value: &T) -> Result<serde_json::Value, PostgresError> {
    serde_json::to_value(value).map_err(PostgresError::internal)
}

pub(super) fn decode_json<T: DeserializeOwned>(
    value: serde_json::Value,
) -> Result<T, PostgresError> {
    serde_json::from_value(value).map_err(PostgresError::internal)
}
