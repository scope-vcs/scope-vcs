use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptCacheKey {
    pub cache_name: String,
    pub compatibility_inputs_digest: String,
    pub exact_inputs_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RestoreAttemptCacheRequest {
    pub cache: AttemptCacheKey,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheRestoreSource {
    Exact,
    Compatible,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum RestoreAttemptCacheResponse {
    Hit {
        source: CacheRestoreSource,
        object_digest: String,
        size_bytes: u64,
        download_url: String,
        expires_at_unix: u64,
    },
    Miss,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrepareAttemptCacheUploadRequest {
    pub cache: AttemptCacheKey,
    pub object_digest: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum PrepareAttemptCacheUploadResponse {
    UseObject {
        object_digest: String,
        expires_at_unix: u64,
    },
    Upload {
        lease_id: String,
        upload_url: String,
        upload_headers: BTreeMap<String, String>,
        expires_at_unix: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommitAttemptCacheUploadRequest {
    pub cache: AttemptCacheKey,
    pub lease_id: String,
    pub object_digest: String,
    pub size_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_transfer_requests_and_responses_keep_their_wire_shape() {
        let cache = AttemptCacheKey {
            cache_name: "cargo".to_string(),
            compatibility_inputs_digest: "c".repeat(64),
            exact_inputs_digest: "e".repeat(64),
        };
        assert_eq!(
            serde_json::to_value(PrepareAttemptCacheUploadRequest {
                cache,
                object_digest: "b".repeat(64),
                size_bytes: 42,
            })
            .unwrap(),
            serde_json::json!({
                "cache": {
                    "cache_name": "cargo",
                    "compatibility_inputs_digest": "c".repeat(64),
                    "exact_inputs_digest": "e".repeat(64),
                },
                "object_digest": "b".repeat(64),
                "size_bytes": 42,
            })
        );
        assert_eq!(
            serde_json::to_value(RestoreAttemptCacheResponse::Miss).unwrap(),
            serde_json::json!({ "result": "miss" })
        );
        assert_eq!(
            serde_json::to_value(PrepareAttemptCacheUploadResponse::Upload {
                lease_id: "lease-1".to_string(),
                upload_url: "https://objects.example/upload".to_string(),
                upload_headers: BTreeMap::from([(
                    "x-amz-meta-scope-sha256".to_string(),
                    "a".repeat(64),
                )]),
                expires_at_unix: 100,
            })
            .unwrap(),
            serde_json::json!({
                "result": "upload",
                "lease_id": "lease-1",
                "upload_url": "https://objects.example/upload",
                "upload_headers": { "x-amz-meta-scope-sha256": "a".repeat(64) },
                "expires_at_unix": 100,
            })
        );
    }
}
