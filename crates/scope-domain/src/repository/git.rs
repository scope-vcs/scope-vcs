use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHead {
    pub head_oid: String,
    pub push_sequence: u64,
    pub change_version: u64,
    pub frontier: GitFrontier,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GitFrontier(String);

impl GitFrontier {
    pub fn from_digest(digest: String) -> Self {
        Self(digest)
    }

    pub fn digest(&self) -> &str {
        &self.0
    }
}

impl GitHead {
    pub fn new(head_oid: String, push_sequence: u64, change_version: u64) -> Self {
        #[derive(Serialize)]
        struct CanonicalFrontierIdentity<'a> {
            version: u8,
            head_oid: &'a str,
            push_sequence: u64,
        }
        let bytes = serde_json::to_vec(&CanonicalFrontierIdentity {
            version: 2,
            head_oid: &head_oid,
            push_sequence,
        })
        .expect("Git frontier fields always serialize");
        Self {
            head_oid,
            push_sequence,
            change_version,
            frontier: GitFrontier(hex::encode(Sha256::digest(bytes))),
        }
    }

    pub fn frontier(&self) -> GitFrontier {
        self.frontier.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSegmentRef {
    pub segment_id: String,
    pub sha256: String,
    pub plaintext_bytes: u64,
    pub encoding_version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitPoolPack {
    pub id: String,
    pub owner: GitPoolPackOwner,
    pub head_oid: String,
    pub prerequisites: Vec<String>,
    pub storage: GitPoolPackStorage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitPoolPackOwner {
    Request {
        request_id: String,
    },
    Main {
        first_sequence: u64,
        last_sequence: u64,
    },
}

#[derive(Clone, PartialEq, Eq)]
pub enum GitPoolPackStorage {
    Inline { sealed: Vec<u8> },
    Segment(GitSegmentRef),
}

impl std::fmt::Debug for GitPoolPackStorage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inline { sealed } => formatter
                .debug_struct("Inline")
                .field("sealed_bytes", &sealed.len())
                .finish(),
            Self::Segment(segment) => formatter.debug_tuple("Segment").field(segment).finish(),
        }
    }
}

impl GitPoolPackStorage {
    pub fn stored_bytes(&self) -> u64 {
        match self {
            Self::Inline { sealed } => sealed.len() as u64,
            Self::Segment(segment) => segment.plaintext_bytes,
        }
    }

    pub fn segment(&self) -> Option<&GitSegmentRef> {
        match self {
            Self::Inline { .. } => None,
            Self::Segment(segment) => Some(segment),
        }
    }
}

impl GitPoolPackOwner {
    pub fn main_sequences(&self) -> Option<(u64, u64)> {
        match self {
            Self::Main {
                first_sequence,
                last_sequence,
            } => Some((*first_sequence, *last_sequence)),
            Self::Request { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MainPackAdvanceError {
    #[error("Git push sequence overflow")]
    SequenceOverflow,
    #[error("Git push does not advance the current main head")]
    NotNextSequence,
    #[error("Git main pack does not cover exactly the pushed sequence")]
    WrongOwner,
    #[error("Git main pack does not end at the pushed head")]
    WrongHead,
    #[error("Git main pack does not build on the previous main head")]
    WrongPrerequisites,
}

pub fn validate_main_pack_advance(
    previous: Option<&GitHead>,
    next: &GitHead,
    pack: &GitPoolPack,
) -> Result<(), MainPackAdvanceError> {
    let sequence = previous
        .map_or(Some(1), |head| head.push_sequence.checked_add(1))
        .ok_or(MainPackAdvanceError::SequenceOverflow)?;
    if next.push_sequence != sequence {
        return Err(MainPackAdvanceError::NotNextSequence);
    }
    if pack.owner
        != (GitPoolPackOwner::Main {
            first_sequence: sequence,
            last_sequence: sequence,
        })
    {
        return Err(MainPackAdvanceError::WrongOwner);
    }
    if pack.head_oid != next.head_oid {
        return Err(MainPackAdvanceError::WrongHead);
    }
    let expected_prerequisites = previous
        .map(|head| vec![head.head_oid.clone()])
        .unwrap_or_default();
    if pack.prerequisites != expected_prerequisites {
        return Err(MainPackAdvanceError::WrongPrerequisites);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitSegmentUploadState {
    Uploading,
    Ready,
    Published,
    Deleting,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSegmentUpload {
    pub segment_id: String,
    pub repository_id: String,
    pub object_key: String,
    pub state: GitSegmentUploadState,
    pub sha256: Option<String>,
    pub plaintext_bytes: Option<u64>,
    pub encrypted_bytes: Option<u64>,
    pub encoding_version: u32,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::{GitPoolPack, GitPoolPackOwner, GitPoolPackStorage, GitSegmentRef};

    pub(crate) fn main_pack(first_sequence: u64, last_sequence: u64) -> GitPoolPack {
        GitPoolPack {
            id: format!("pack-{first_sequence}-{last_sequence}"),
            owner: GitPoolPackOwner::Main {
                first_sequence,
                last_sequence,
            },
            head_oid: format!("head-{last_sequence}"),
            prerequisites: (first_sequence > 1)
                .then(|| format!("head-{}", first_sequence - 1))
                .into_iter()
                .collect(),
            storage: GitPoolPackStorage::Segment(segment(first_sequence, last_sequence)),
        }
    }

    pub(crate) fn segment(first_sequence: u64, last_sequence: u64) -> GitSegmentRef {
        GitSegmentRef {
            segment_id: format!("segment-{first_sequence}-{last_sequence}"),
            sha256: format!("{first_sequence:032x}{last_sequence:032x}"),
            plaintext_bytes: 1,
            encoding_version: 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::main_pack;
    use super::*;

    #[test]
    fn frontier_preserves_the_signed_identity_and_ignores_non_git_changes() {
        let head = GitHead::new("a".repeat(40), 1, 7);
        assert_eq!(
            head.frontier.digest(),
            "189efc3e1b3dd4adb49c858cdb3be61d3a0ebafe1d39fa3e9b8a1a9c7461e6e9"
        );
        assert_eq!(
            head.frontier(),
            GitHead::new("a".repeat(40), 1, 8).frontier()
        );
        assert_ne!(
            head.frontier(),
            GitHead::new("a".repeat(40), 2, 8).frontier()
        );
        assert_ne!(
            head.frontier(),
            GitHead::new("b".repeat(40), 1, 7).frontier()
        );
    }

    #[test]
    fn persisted_frontier_is_not_recomputed_when_loading_historic_heads() {
        let stored = serde_json::json!({
            "head_oid": "a".repeat(40),
            "push_sequence": 1,
            "change_version": 7,
            "frontier": "b".repeat(64),
        });
        let head: GitHead = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(head.frontier.digest(), "b".repeat(64));
        assert_eq!(serde_json::to_value(head).unwrap(), stored);
    }

    #[test]
    fn a_main_pack_advances_exactly_one_push_from_the_previous_head() {
        let previous = GitHead::new("head-4".to_string(), 4, 9);
        let next = GitHead::new("head-5".to_string(), 5, 10);
        validate_main_pack_advance(Some(&previous), &next, &main_pack(5, 5)).unwrap();
        validate_main_pack_advance(
            None,
            &GitHead::new("head-1".to_string(), 1, 1),
            &main_pack(1, 1),
        )
        .unwrap();

        let mut unrelated = main_pack(5, 5);
        unrelated.prerequisites = vec!["head-3".to_string()];
        let mut wrong_head = main_pack(5, 5);
        wrong_head.head_oid = "head-6".to_string();
        let mut request_owned = main_pack(5, 5);
        request_owned.owner = GitPoolPackOwner::Request {
            request_id: "request".to_string(),
        };
        for (pack, error) in [
            (unrelated, MainPackAdvanceError::WrongPrerequisites),
            (wrong_head, MainPackAdvanceError::WrongHead),
            (main_pack(4, 5), MainPackAdvanceError::WrongOwner),
            (request_owned, MainPackAdvanceError::WrongOwner),
        ] {
            assert_eq!(
                validate_main_pack_advance(Some(&previous), &next, &pack),
                Err(error)
            );
        }
        assert_eq!(
            validate_main_pack_advance(
                Some(&previous),
                &GitHead::new("head-6".to_string(), 6, 10),
                &main_pack(6, 6)
            ),
            Err(MainPackAdvanceError::NotNextSequence)
        );
        let mut first_with_base = main_pack(1, 1);
        first_with_base.prerequisites = vec!["head-0".to_string()];
        assert_eq!(
            validate_main_pack_advance(
                None,
                &GitHead::new("head-1".to_string(), 1, 1),
                &first_with_base
            ),
            Err(MainPackAdvanceError::WrongPrerequisites)
        );
    }
}
