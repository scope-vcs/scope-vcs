use scope_media_storage::{MAX_CHUNK_BYTES, MediaObject, MediaStorage, WriteAttempt};
use sha2::{Digest, Sha256};

pub async fn store_bytes(
    storage: &MediaStorage,
    attempt: &WriteAttempt,
    bytes: &[u8],
) -> MediaObject {
    let mut parts = Vec::new();
    for (index, chunk) in bytes.chunks(MAX_CHUNK_BYTES).enumerate() {
        let part = storage.plan_part(attempt, index as u32 + 1, chunk).unwrap();
        storage.write_part(&part, chunk.to_vec()).await.unwrap();
        parts.push(part);
    }
    storage
        .seal_parts(
            "video/mp4",
            bytes.len() as u64,
            &hex::encode(Sha256::digest(bytes)),
            parts,
        )
        .await
        .unwrap()
}
