use crate::{
    GitSegmentStore,
    envelope::{DecryptedFrame, EnvelopeReader, EnvelopeScope, seal},
    error::GitStorageError,
};

const INLINE_PACK_FRAME_BYTES: usize = 256 * 1024;

impl GitSegmentStore {
    pub fn seal_inline_pack(
        &self,
        repository_id: &str,
        pack_id: &str,
        pack: Vec<u8>,
    ) -> Result<Vec<u8>, GitStorageError> {
        seal(
            &self.encryption_key,
            EnvelopeScope::git_segment(repository_id, pack_id),
            INLINE_PACK_FRAME_BYTES,
            pack,
        )
    }

    pub async fn open_inline_pack(
        &self,
        repository_id: &str,
        pack_id: &str,
        sealed: &[u8],
    ) -> Result<Vec<u8>, GitStorageError> {
        let mut source = sealed;
        let mut envelope = EnvelopeReader::read_header(
            &mut source,
            &self.encryption_key,
            EnvelopeScope::git_segment(repository_id, pack_id),
        )
        .await?;
        let mut pack = Vec::new();
        while let DecryptedFrame::Data(frame) = envelope.next(&mut source).await? {
            pack.extend_from_slice(&frame);
        }
        if !source.is_empty() {
            return Err(GitStorageError::InvalidEnvelope(
                "inline pack has bytes after its final frame".into(),
            ));
        }
        Ok(pack)
    }
}

#[cfg(test)]
mod tests {
    use crate::{EncryptionKey, GitSegmentStore, GitSegmentStoreConfig, MemoryBackend};
    use std::sync::Arc;

    fn store() -> GitSegmentStore {
        GitSegmentStore::new(
            Arc::new(MemoryBackend::default()),
            EncryptionKey::new("test", [3_u8; 32]).unwrap(),
            GitSegmentStoreConfig::new(tempfile::tempdir().unwrap().keep()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn an_inline_pack_opens_only_for_its_repository_and_id() {
        let store = store();
        let pack = (0..700_000_u32)
            .map(|value| value as u8)
            .collect::<Vec<_>>();
        let sealed = store
            .seal_inline_pack("owner/repo", "pack-1", pack.clone())
            .unwrap();

        assert_ne!(sealed, pack);
        assert_eq!(
            store
                .open_inline_pack("owner/repo", "pack-1", &sealed)
                .await
                .unwrap(),
            pack
        );
        assert!(
            store
                .open_inline_pack("owner/repo", "pack-2", &sealed)
                .await
                .is_err()
        );
        assert!(
            store
                .open_inline_pack("owner/other", "pack-1", &sealed)
                .await
                .is_err()
        );
    }
}
