use crate::object_store_config;
use scope_object_store::{EncryptedObjectStore, ObjectStore};
use scope_postgres::db::GitSegmentV1Cleanup;
use std::sync::Arc;

pub async fn cleanup_git_segments_v1_for_maintenance(
    database_url: String,
) -> anyhow::Result<usize> {
    let cleanup = GitSegmentV1Cleanup::begin(database_url).await?;
    let encryption_key = object_store_config::encryption_key_from_env()?;
    let legacy_store = legacy_store_from_env(encryption_key).await?;
    let objects = cleanup.legacy_objects().await?;
    for object in &objects {
        legacy_store.delete(&format!("objects/git-segments/{}", object.sha256))?;
        cleanup.remove_record(object).await?;
    }
    Ok(objects.len())
}

async fn legacy_store_from_env(encryption_key: [u8; 32]) -> anyhow::Result<Arc<dyn ObjectStore>> {
    #[cfg(feature = "local-dev")]
    if crate::dev::is_local_dev_env() {
        let root = crate::config::data_dir(&crate::config::git_repo_root()).join("objects");
        return Ok(Arc::new(EncryptedObjectStore::new(
            Arc::new(object_store_config::file_from_env(&root)),
            encryption_key,
        )));
    }
    let s3 = tokio::task::spawn_blocking(object_store_config::s3_from_env).await??;
    Ok(Arc::new(EncryptedObjectStore::new(
        Arc::new(s3),
        encryption_key,
    )))
}
