use super::{ObjectStore, ensure_object_size, object_too_large};
use crate::ObjectStoreError;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::PathBuf,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileObjectStoreSettings {
    pub root: PathBuf,
}

impl FileObjectStoreSettings {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

pub struct FileObjectStore {
    root: PathBuf,
}

impl FileObjectStore {
    pub fn new(settings: FileObjectStoreSettings) -> Self {
        Self {
            root: settings.root,
        }
    }

    fn path_for_key(&self, key: &str) -> PathBuf {
        let digest = hex::encode(Sha256::digest(key.as_bytes()));
        self.root.join(&digest[..2]).join(digest)
    }

    fn ensure_root(&self) -> Result<(), ObjectStoreError> {
        std::fs::create_dir_all(&self.root).map_err(|error| {
            ObjectStoreError::service_unavailable(format!(
                "failed to prepare local object store {}: {error}",
                self.root.display()
            ))
        })
    }
}

impl ObjectStore for FileObjectStore {
    fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), ObjectStoreError> {
        let path = self.path_for_key(key);
        let parent = path.parent().ok_or_else(|| {
            ObjectStoreError::internal_message("local object path is missing a parent directory")
        })?;
        std::fs::create_dir_all(parent).map_err(|error| {
            ObjectStoreError::service_unavailable(format!(
                "failed to prepare local object directory {}: {error}",
                parent.display()
            ))
        })?;
        std::fs::write(&path, bytes).map_err(|error| {
            ObjectStoreError::service_unavailable(format!(
                "failed to write local object {}: {error}",
                path.display()
            ))
        })
    }

    fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, ObjectStoreError> {
        let path = self.path_for_key(key);
        let file = File::open(&path).map_err(|error| read_error(key, error))?;
        let length = file
            .metadata()
            .map_err(|error| read_error(key, error))?
            .len();
        read_bounded(file, key, length, max_bytes)
    }

    fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        let path = self.path_for_key(key);
        match std::fs::remove_file(&path) {
            Ok(()) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::remove_dir(parent);
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ObjectStoreError::service_unavailable(format!(
                "failed to delete local object {}: {error}",
                path.display()
            ))),
        }
    }

    fn readiness_check(&self) -> Result<(), ObjectStoreError> {
        self.ensure_root()
    }
}

fn read_error(key: &str, error: io::Error) -> ObjectStoreError {
    if error.kind() == io::ErrorKind::NotFound {
        ObjectStoreError::not_found(format!("object {key} not found"))
    } else {
        ObjectStoreError::service_unavailable(format!("failed to read object {key}: {error}"))
    }
}

fn read_bounded(
    file: File,
    key: &str,
    length: u64,
    max_bytes: usize,
) -> Result<Vec<u8>, ObjectStoreError> {
    if length > max_bytes as u64 {
        return Err(object_too_large(
            "read",
            key,
            usize::try_from(length).unwrap_or(usize::MAX),
            max_bytes,
        ));
    }
    // The file may grow after metadata is read. Never rely on that snapshot as the read cap.
    let mut bytes = Vec::new();
    file.take((max_bytes as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| read_error(key, error))?;
    ensure_object_size("read", key, bytes.len(), max_bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EncryptedObjectStore, MemoryObjectStore, ObjectStoreErrorKind};
    use std::{
        io::{Seek, Write},
        sync::Arc,
    };

    struct Fixture(FileObjectStore);

    impl Fixture {
        fn new() -> Self {
            let mut nonce = [0; 16];
            getrandom::fill(&mut nonce).unwrap();
            Self(FileObjectStore::new(FileObjectStoreSettings::new(
                std::env::temp_dir().join(format!("scope-file-store-{}", hex::encode(nonce))),
            )))
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0.root);
        }
    }

    #[test]
    fn file_store_round_trips_and_deletes_by_object_key_hash() {
        let fixture = Fixture::new();
        let store = &fixture.0;

        store.put("../not-a-path", b"payload".to_vec()).unwrap();
        assert_eq!(store.get("../not-a-path").unwrap(), b"payload");
        store.delete("../not-a-path").unwrap();
        assert!(store.get("../not-a-path").is_err());
    }

    #[test]
    fn local_and_memory_reads_agree_on_limits_and_absence() {
        let fixture = Fixture::new();
        let memory = MemoryObjectStore::new();
        for store in [&fixture.0 as &dyn ObjectStore, &memory] {
            store.put("payload", b"four".to_vec()).unwrap();
            assert_eq!(store.get_bounded("payload", 4).unwrap(), b"four");
            assert_eq!(
                store.get_bounded("payload", 3).unwrap_err().kind,
                ObjectStoreErrorKind::PayloadTooLarge
            );
            assert_eq!(
                store.get_bounded("missing", 4).unwrap_err().kind,
                ObjectStoreErrorKind::NotFound
            );
            store.put("empty", Vec::new()).unwrap();
            assert!(store.get_bounded("empty", 0).unwrap().is_empty());
        }
    }

    #[test]
    fn local_io_failure_is_not_reported_as_missing_content() {
        let fixture = Fixture::new();
        // A directory in place of an object fails even when tests run as root.
        std::fs::create_dir_all(fixture.0.path_for_key("unreadable")).unwrap();
        assert_eq!(
            fixture.0.get("unreadable").unwrap_err().kind,
            ObjectStoreErrorKind::ServiceUnavailable
        );
        assert_eq!(
            read_error("key", io::ErrorKind::PermissionDenied.into()).kind,
            ObjectStoreErrorKind::ServiceUnavailable
        );
    }

    #[test]
    fn sparse_object_is_rejected_before_reading_and_growth_is_still_capped() {
        let fixture = Fixture::new();
        let store = &fixture.0;
        store.put("growing", b"four".to_vec()).unwrap();
        let mut file = File::open(store.path_for_key("growing")).unwrap();
        let initial_length = file.metadata().unwrap().len();
        let mut writer = std::fs::OpenOptions::new()
            .write(true)
            .open(store.path_for_key("growing"))
            .unwrap();
        writer.set_len(1024 * 1024 * 1024).unwrap();
        writer.write_all(b"five!").unwrap();

        let error =
            read_bounded(file.try_clone().unwrap(), "growing", initial_length, 4).unwrap_err();
        assert_eq!(error.kind, ObjectStoreErrorKind::PayloadTooLarge);
        assert_eq!(
            file.stream_position().unwrap(),
            5,
            "read cap must survive growth after metadata inspection"
        );
        file.rewind().unwrap();
        let error = read_bounded(
            file.try_clone().unwrap(),
            "growing",
            writer.metadata().unwrap().len(),
            4,
        )
        .unwrap_err();
        assert_eq!(error.kind, ObjectStoreErrorKind::PayloadTooLarge);
        assert_eq!(
            file.stream_position().unwrap(),
            0,
            "known oversized objects must not be read"
        );
        assert_eq!(
            store.get_bounded("growing", 4).unwrap_err().kind,
            ObjectStoreErrorKind::PayloadTooLarge
        );
    }

    #[test]
    fn encrypted_reads_include_only_the_envelope_allowance() {
        let fixture = Fixture::new();
        let file: Arc<dyn ObjectStore> = Arc::new(FileObjectStore::new(
            FileObjectStoreSettings::new(fixture.0.root.clone()),
        ));
        for raw in [file, Arc::new(MemoryObjectStore::new())] {
            let encrypted = EncryptedObjectStore::new(raw, [7; 32]);
            encrypted.put("encrypted", b"four".to_vec()).unwrap();
            assert_eq!(encrypted.get_bounded("encrypted", 4).unwrap(), b"four");
            assert_eq!(
                encrypted.get_bounded("encrypted", 3).unwrap_err().kind,
                ObjectStoreErrorKind::PayloadTooLarge
            );
        }
    }
}
