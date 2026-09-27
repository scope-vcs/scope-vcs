use crate::{
    BackendError, MultipartUpload, ObjectBackend, RemoteReader, UploadedPart, is_hex_id_32,
    random_hex_id, sync_directory,
};
use async_trait::async_trait;
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
};

#[derive(Clone, Debug)]
pub struct FileBackend {
    root: PathBuf,
}

impl FileBackend {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, BackendError> {
        let root = root.into();
        if root.as_os_str().is_empty() {
            return Err(BackendError::new("filesystem multipart root is required"));
        }
        Ok(Self { root })
    }

    fn uploads_root(&self) -> PathBuf {
        self.root.join("multipart")
    }

    fn upload_path(&self, upload_id: &str) -> Result<PathBuf, BackendError> {
        validate_upload_id(upload_id)?;
        Ok(self.uploads_root().join(upload_id))
    }

    fn object_path(&self, key: &str) -> Result<PathBuf, BackendError> {
        validate_key(key)?;
        Ok(self.root.join("objects").join(key))
    }

    async fn verify_upload(&self, upload: &MultipartUpload) -> Result<PathBuf, BackendError> {
        validate_key(&upload.key)?;
        let directory = self.upload_path(&upload.upload_id)?;
        let recorded_key = fs::read_to_string(directory.join("key")).await?;
        if recorded_key != upload.key {
            return Err(BackendError::new(
                "filesystem multipart upload key does not match",
            ));
        }
        Ok(directory)
    }
}

#[async_trait]
impl ObjectBackend for FileBackend {
    /// Replaces any existing object, like an S3 put.
    async fn put(&self, key: &str, bytes: Bytes) -> Result<(), BackendError> {
        let final_path = self.object_path(key)?;
        let parent = final_path
            .parent()
            .ok_or_else(|| BackendError::new("filesystem object path has no parent"))?
            .to_path_buf();
        fs::create_dir_all(&parent).await?;
        fs::create_dir_all(self.uploads_root()).await?;
        let temp_path = self
            .uploads_root()
            .join(format!("{}.put", random_upload_id()?));
        let result = async {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp_path)
                .await?;
            file.write_all(&bytes).await?;
            file.sync_all().await?;
            drop(file);
            fs::rename(&temp_path, &final_path).await?;
            sync_directory(parent).await.map_err(BackendError::from)
        }
        .await;
        if result.is_err() {
            let _ = fs::remove_file(&temp_path).await;
        }
        result
    }

    async fn begin(&self, key: &str) -> Result<MultipartUpload, BackendError> {
        validate_key(key)?;
        fs::create_dir_all(self.uploads_root()).await?;
        let upload_id = random_upload_id()?;
        let directory = self.upload_path(&upload_id)?;
        fs::create_dir(&directory).await?;
        let mut metadata = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join("key"))
            .await
            .map_err(BackendError::from)?;
        metadata
            .write_all(key.as_bytes())
            .await
            .map_err(BackendError::from)?;
        metadata.sync_all().await.map_err(BackendError::from)?;
        sync_directory(directory.clone())
            .await
            .map_err(BackendError::from)?;
        Ok(MultipartUpload {
            key: key.to_string(),
            upload_id,
        })
    }

    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: i32,
        bytes: Bytes,
    ) -> Result<UploadedPart, BackendError> {
        if part_number <= 0 {
            return Err(BackendError::new("multipart part number must be positive"));
        }
        let directory = self.verify_upload(upload).await?;
        let part_name = format!("{part_number:08}.part");
        let temp_path = directory.join(format!("{part_name}.tmp"));
        let final_path = directory.join(part_name);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await?;
        let result = async {
            file.write_all(&bytes).await?;
            file.sync_all().await?;
            drop(file);
            if fs::try_exists(&final_path).await? {
                return Err(BackendError::new(
                    "filesystem multipart part already exists",
                ));
            }
            fs::rename(&temp_path, &final_path)
                .await
                .map_err(BackendError::from)?;
            sync_directory(directory).await.map_err(BackendError::from)
        }
        .await;
        if let Err(error) = result {
            let _ = fs::remove_file(&temp_path).await;
            return Err(error);
        }
        Ok(UploadedPart {
            part_number,
            etag: hex::encode(Sha256::digest(&bytes)),
        })
    }

    async fn complete(
        &self,
        upload: MultipartUpload,
        parts: Vec<UploadedPart>,
    ) -> Result<(), BackendError> {
        if parts.is_empty() {
            return Err(BackendError::new(
                "filesystem multipart upload has no parts",
            ));
        }
        let upload_directory = self.verify_upload(&upload).await?;
        let final_path = self.object_path(&upload.key)?;
        let parent = final_path
            .parent()
            .ok_or_else(|| BackendError::new("filesystem object path has no parent"))?;
        fs::create_dir_all(parent).await?;
        if fs::try_exists(&final_path).await? {
            return Err(BackendError::new(
                "filesystem multipart object already exists",
            ));
        }
        let temp_path = upload_directory.join("object.tmp");
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await?;
        let result = async {
            for (index, part) in parts.iter().enumerate() {
                let expected = i32::try_from(index + 1)
                    .map_err(|_| BackendError::new("multipart part count exceeds i32"))?;
                if part.part_number != expected {
                    return Err(BackendError::new(
                        "filesystem multipart parts are not contiguous",
                    ));
                }
                let path = upload_directory.join(format!("{:08}.part", part.part_number));
                let mut input = File::open(path).await?;
                tokio::io::copy(&mut input, &mut output).await?;
            }
            output.sync_all().await?;
            drop(output);
            fs::rename(&temp_path, &final_path)
                .await
                .map_err(BackendError::from)?;
            sync_directory(parent.to_path_buf())
                .await
                .map_err(BackendError::from)?;
            let _ = fs::remove_dir_all(&upload_directory).await;
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = fs::remove_file(&temp_path).await;
        }
        result
    }

    async fn abort(&self, upload: MultipartUpload) -> Result<(), BackendError> {
        let directory = self.verify_upload(&upload).await?;
        fs::remove_dir_all(directory)
            .await
            .map_err(BackendError::from)
    }

    async fn abort_incomplete(&self, key: &str) -> Result<(), BackendError> {
        validate_key(key)?;
        let uploads_root = self.uploads_root();
        let mut entries = match fs::read_dir(&uploads_root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(BackendError::from(error)),
        };
        while let Some(entry) = entries.next_entry().await? {
            let Some(upload_id) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                continue;
            };
            if validate_upload_id(&upload_id).is_err() {
                continue;
            }
            let directory = self.upload_path(&upload_id)?;
            let recorded_key = match fs::read_to_string(directory.join("key")).await {
                Ok(recorded_key) => recorded_key,
                Err(_) => continue,
            };
            if recorded_key == key {
                fs::remove_dir_all(directory).await?;
            }
        }
        Ok(())
    }

    async fn read(&self, key: &str) -> Result<RemoteReader, BackendError> {
        let path = self.object_path(key)?;
        let file = File::open(path).await?;
        Ok(Box::pin(file))
    }

    async fn delete(&self, key: &str) -> Result<(), BackendError> {
        let path = self.object_path(key)?;
        match fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(BackendError::from(error)),
        }
    }

    /// Proves the objects directory exists or can be created, which fails when the path is a
    /// file or its parent is not writable.
    async fn readiness_check(&self) -> Result<(), BackendError> {
        fs::create_dir_all(self.root.join("objects")).await?;
        Ok(())
    }
}

fn validate_key(key: &str) -> Result<(), BackendError> {
    if key.split('/').any(|component| {
        component.is_empty()
            || component == "."
            || component == ".."
            || !component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    }) {
        return Err(BackendError::new("filesystem object key is invalid"));
    }
    Ok(())
}

fn validate_upload_id(upload_id: &str) -> Result<(), BackendError> {
    if !is_hex_id_32(upload_id) {
        return Err(BackendError::new(
            "filesystem multipart upload id is invalid",
        ));
    }
    Ok(())
}

fn random_upload_id() -> Result<String, BackendError> {
    random_hex_id()
        .map_err(|error| BackendError::new(format!("creating multipart upload id: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn readiness_fails_when_the_objects_directory_cannot_exist() {
        let root = tempfile::tempdir().unwrap();
        FileBackend::new(root.path())
            .unwrap()
            .readiness_check()
            .await
            .unwrap();

        let blocked = tempfile::tempdir().unwrap();
        std::fs::write(blocked.path().join("objects"), b"not a directory").unwrap();
        assert!(
            FileBackend::new(blocked.path())
                .unwrap()
                .readiness_check()
                .await
                .is_err()
        );
    }
}
