use crate::{GitSnapshotError, GitStorageLimits};
use scope_domain::repository::git::{GitHead, GitPackSpan, GitSegmentRef};
use scope_object_store::ensure_object_size;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredGitPush {
    pub head: GitHead,
    pub pack_span: GitPackSpan,
}

pub fn prepare_git_push(
    segment: GitSegmentRef,
    head_oid: String,
    previous: Option<&GitHead>,
    storage_limits: GitStorageLimits,
) -> Result<StoredGitPush, GitSnapshotError> {
    ensure_object_size(
        "write",
        "Git pack",
        usize::try_from(segment.plaintext_bytes).unwrap_or(usize::MAX),
        storage_limits.max_object_bytes(),
    )?;
    let sequence = storage_limits.next_push_sequence(previous.map(|head| head.push_sequence))?;
    Ok(StoredGitPush {
        head: GitHead::new(
            head_oid.clone(),
            sequence,
            previous.map_or(1, |head| head.change_version.saturating_add(1)),
        ),
        pack_span: GitPackSpan {
            first_sequence: sequence,
            last_sequence: sequence,
            geometric_tier: 0,
            base_oid: previous.map(|head| head.head_oid.clone()),
            head_oid,
            segment,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment() -> GitSegmentRef {
        GitSegmentRef {
            segment_id: "segment-5".to_string(),
            sha256: "pack-sha".to_string(),
            plaintext_bytes: 4,
            encoding_version: 2,
        }
    }

    #[test]
    fn push_preparation_separates_snapshot_identity_from_pack_layout() {
        let previous = GitHead::new("head-1".to_string(), 4, 9);

        let stored = prepare_git_push(
            segment(),
            "head-2".to_string(),
            Some(&previous),
            GitStorageLimits::new(4096).unwrap(),
        )
        .unwrap();

        assert_eq!(stored.head.push_sequence, 5);
        assert_eq!(stored.head.change_version, 10);
        assert_eq!(stored.pack_span.first_sequence, 5);
        assert_eq!(stored.pack_span.last_sequence, 5);
        assert_eq!(stored.pack_span.base_oid.as_deref(), Some("head-1"));
        assert_eq!(stored.pack_span.head_oid, "head-2");
        assert_eq!(stored.pack_span.segment, segment());
    }

    #[test]
    fn preparation_rejects_segment_larger_than_storage_limit() {
        let error = prepare_git_push(
            segment(),
            "head".to_string(),
            None,
            GitStorageLimits::new(3).unwrap(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("exceeds 3 bytes"));
    }
}
