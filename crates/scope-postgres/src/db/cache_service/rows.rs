use super::{CacheObjectRecord, CacheUploadRecord, CacheUploadState};
use crate::{db::integer_columns::i64_to_u64, error::PostgresError};
use scope_cache_domain::{
    CacheDigest, CacheObject, CacheReference, RepositoryId, UploadLease, UploadLeaseId,
};
use sea_orm::{FromQueryResult, QueryResult};

#[derive(Clone, Debug)]
pub(super) struct ReferenceRow {
    pub(super) checksum_sha256: String,
    pub(super) compatibility_group_digest: String,
    pub(super) last_accessed_at_unix: u64,
    pub(super) expires_at_unix: u64,
}

#[derive(FromQueryResult)]
struct StoredReferenceRow {
    checksum_sha256: String,
    compatibility_group_digest: String,
    last_accessed_at_unix: i64,
    expires_at_unix: i64,
}

pub(super) fn decode_reference(row: &QueryResult) -> Result<ReferenceRow, PostgresError> {
    let row = StoredReferenceRow::from_query_result(row, "")?;
    Ok(ReferenceRow {
        checksum_sha256: row.checksum_sha256,
        compatibility_group_digest: row.compatibility_group_digest,
        last_accessed_at_unix: i64_to_u64(
            row.last_accessed_at_unix,
            "cache reference last access time",
        )?,
        expires_at_unix: i64_to_u64(row.expires_at_unix, "cache expiry")?,
    })
}

#[derive(FromQueryResult)]
struct ObjectRow {
    repository_id: String,
    checksum_sha256: String,
    storage_backend: String,
    object_key: String,
    size_bytes: i64,
    created_at_unix: i64,
}

#[derive(FromQueryResult)]
struct UploadRow {
    upload_id: String,
    repository_id: String,
    identity_digest: String,
    compatibility_group_digest: String,
    checksum_sha256: String,
    storage_backend: String,
    object_key: String,
    size_bytes: i64,
    state: String,
    created_at_unix: i64,
    expires_at_unix: i64,
}

pub(super) fn decode_object(row: &QueryResult) -> Result<CacheObjectRecord, PostgresError> {
    let row = ObjectRow::from_query_result(row, "")?;
    Ok(CacheObjectRecord {
        repository_id: row.repository_id,
        checksum_sha256: row.checksum_sha256,
        storage_backend: row.storage_backend,
        object_key: row.object_key,
        size_bytes: i64_to_u64(row.size_bytes, "cache object size")?,
        created_at_unix: i64_to_u64(row.created_at_unix, "cache object creation time")?,
    })
}

pub(super) fn decode_upload(row: &QueryResult) -> Result<CacheUploadRecord, PostgresError> {
    let row = UploadRow::from_query_result(row, "")?;
    Ok(CacheUploadRecord {
        upload_id: row.upload_id,
        repository_id: row.repository_id,
        identity_digest: row.identity_digest,
        compatibility_group_digest: row.compatibility_group_digest,
        checksum_sha256: row.checksum_sha256,
        storage_backend: row.storage_backend,
        object_key: row.object_key,
        size_bytes: i64_to_u64(row.size_bytes, "cache object size")?,
        state: match row.state.as_str() {
            "active" => CacheUploadState::Active,
            "deleting" => CacheUploadState::Deleting,
            "committed" => CacheUploadState::Committed,
            value => {
                return Err(PostgresError::internal_message(format!(
                    "unknown cache upload state {value}"
                )));
            }
        },
        created_at_unix: i64_to_u64(row.created_at_unix, "cache object creation time")?,
        expires_at_unix: i64_to_u64(row.expires_at_unix, "cache expiry")?,
    })
}

pub(super) fn domain_reference_from_query(
    repository_id: &str,
    identity_digest: &str,
    row: &QueryResult,
) -> Result<CacheReference, PostgresError> {
    domain_reference(repository_id, identity_digest, &decode_reference(row)?)
}

pub(super) fn domain_object(record: &CacheObjectRecord) -> Result<CacheObject, PostgresError> {
    CacheObject::new(
        RepositoryId::parse(record.repository_id.clone())?,
        CacheDigest::parse(record.checksum_sha256.clone())?,
        record.size_bytes,
        record.created_at_unix,
    )
    .map_err(PostgresError::from)
}

pub(super) fn domain_reference(
    repository_id: &str,
    identity_digest: &str,
    row: &ReferenceRow,
) -> Result<CacheReference, PostgresError> {
    CacheReference::restore(
        RepositoryId::parse(repository_id.to_string())?,
        CacheDigest::parse(identity_digest.to_string())?,
        CacheDigest::parse(row.compatibility_group_digest.clone())?,
        CacheDigest::parse(row.checksum_sha256.clone())?,
        row.last_accessed_at_unix,
        row.expires_at_unix,
    )
    .map_err(PostgresError::from)
}

pub(super) fn domain_upload(record: &CacheUploadRecord) -> Result<UploadLease, PostgresError> {
    UploadLease::restore(
        UploadLeaseId::parse(record.upload_id.clone())?,
        RepositoryId::parse(record.repository_id.clone())?,
        CacheDigest::parse(record.identity_digest.clone())?,
        CacheDigest::parse(record.compatibility_group_digest.clone())?,
        CacheDigest::parse(record.checksum_sha256.clone())?,
        record.size_bytes,
        record.created_at_unix,
        record.expires_at_unix,
    )
    .map_err(PostgresError::from)
}
