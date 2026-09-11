use scope_media_storage::{MediaStorage, MediaStorageSettings};

pub async fn from_env() -> anyhow::Result<MediaStorage> {
    Ok(MediaStorageSettings::from_env()?.connect(2).await?)
}
