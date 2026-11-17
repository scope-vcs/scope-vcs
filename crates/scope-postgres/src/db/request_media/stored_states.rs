use crate::{db::entities::decode_enum, error::PostgresError};
use sea_orm::QueryResult;
use serde::{Deserialize, de::DeserializeOwned};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub(super) enum UploadPartState {
    Pending,
    Stored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub(super) enum ProcessingJobState {
    Queued,
    Leased,
    Completed,
    Canceled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub(super) enum ProcessingObjectState {
    Pending,
    Adopted,
    Orphaned,
    Deleted,
}

pub(super) fn stored_state<T: DeserializeOwned>(row: &QueryResult) -> Result<T, PostgresError> {
    decode_enum(row.try_get::<String>("", "state")?)
}
