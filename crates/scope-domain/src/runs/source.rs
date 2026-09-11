use super::validation::{validate_git_oid, validate_sha256_hash};
use crate::{
    content::SourceBlob,
    content_ref::ContentRef,
    error::DomainError,
    projection::ProjectionViewKey,
    repository::git::{GitHead, GitPackSpan, GitSegmentRef, validate_git_pack_layout},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunTrigger {
    Manual,
    PushMain,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RunSource {
    EphemeralGitBundle {
        object: SourceBlob,
    },
    AcceptedGitHead {
        repository_id: String,
        head: GitHead,
        pack_spans: Vec<GitPackSpan>,
        audience: ProjectionViewKey,
    },
}

impl RunSource {
    pub fn ephemeral_git_bundle(object: SourceBlob) -> Result<Self, DomainError> {
        validate_source_blob(&object, "run source bundle")?;
        if !matches!(object.content_ref, ContentRef::GitBundleSha256(_)) {
            return Err(DomainError::invalid_input(
                "ephemeral run source must be a Git bundle",
            ));
        }
        Ok(Self::EphemeralGitBundle { object })
    }

    pub fn accepted_git_head(
        repository_id: impl Into<String>,
        head: GitHead,
        pack_spans: Vec<GitPackSpan>,
        audience: ProjectionViewKey,
    ) -> Result<Self, DomainError> {
        let repository_id = repository_id.into();
        if repository_id.trim().is_empty() {
            return Err(DomainError::invalid_input(
                "accepted run source repository id is required",
            ));
        }
        if head.push_sequence == 0 || head.change_version == 0 {
            return Err(DomainError::invalid_input(
                "accepted run source sequence and change version must be positive",
            ));
        }
        validate_git_oid("accepted run source head", &head.head_oid)?;
        validate_sha256_hash("accepted run source frontier", head.frontier.digest())?;
        if pack_spans.is_empty() {
            return Err(DomainError::invalid_input(
                "accepted run source must pin at least one Git pack span",
            ));
        }
        validate_git_pack_layout(&pack_spans)
            .map_err(|error| DomainError::invalid_input(error.to_string()))?;
        let final_span = pack_spans.last().expect("empty pack spans were rejected");
        if final_span.last_sequence != head.push_sequence || final_span.head_oid != head.head_oid {
            return Err(DomainError::invalid_input(
                "accepted run source packs do not reach the accepted head",
            ));
        }
        Ok(Self::AcceptedGitHead {
            repository_id,
            head,
            pack_spans,
            audience,
        })
    }

    pub fn ephemeral_bundle(&self) -> Option<&SourceBlob> {
        match self {
            Self::EphemeralGitBundle { object } => Some(object),
            Self::AcceptedGitHead { .. } => None,
        }
    }

    pub fn retained_objects(&self) -> Vec<&SourceBlob> {
        match self {
            Self::EphemeralGitBundle { object } => vec![object],
            Self::AcceptedGitHead { .. } => Vec::new(),
        }
    }

    pub fn retained_git_segments(&self) -> Vec<&GitSegmentRef> {
        match self {
            Self::EphemeralGitBundle { .. } => Vec::new(),
            Self::AcceptedGitHead { pack_spans, .. } => {
                pack_spans.iter().map(|span| &span.segment).collect()
            }
        }
    }

    pub fn source_identity(&self) -> &str {
        match self {
            Self::EphemeralGitBundle { object } => &object.sha256,
            Self::AcceptedGitHead { head, .. } => head.frontier.digest(),
        }
    }

    pub fn git_oid(&self) -> &str {
        match self {
            Self::EphemeralGitBundle { object } => &object.git_oid,
            Self::AcceptedGitHead { head, .. } => &head.head_oid,
        }
    }

    pub fn logical_git_head(&self) -> Option<(&str, &GitHead, &[GitPackSpan])> {
        match self {
            Self::AcceptedGitHead {
                repository_id,
                head,
                pack_spans,
                ..
            } => Some((repository_id, head, pack_spans)),
            Self::EphemeralGitBundle { .. } => None,
        }
    }
}

fn validate_source_blob(blob: &SourceBlob, label: &str) -> Result<(), DomainError> {
    validate_sha256_hash(&format!("{label} digest"), &blob.sha256)?;
    if blob.content_ref.sha256() != Some(blob.sha256.as_str()) {
        return Err(DomainError::invalid_input(format!(
            "{label} content reference does not match its digest"
        )));
    }
    validate_git_oid(&format!("{label} Git OID"), &blob.git_oid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_git_head_pins_the_exact_pack_layout() {
        let head_oid = "a".repeat(40);
        let pack = GitPackSpan {
            first_sequence: 1,
            last_sequence: 1,
            geometric_tier: 0,
            base_oid: None,
            head_oid: head_oid.clone(),
            segment: git_segment('c'),
        };
        let source = RunSource::accepted_git_head(
            "owner/repo",
            GitHead::new(head_oid.clone(), 1, 7),
            vec![pack.clone()],
            ProjectionViewKey::Private,
        )
        .unwrap();

        assert_eq!(source.git_oid(), head_oid);
        assert_eq!(
            source.source_identity(),
            "189efc3e1b3dd4adb49c858cdb3be61d3a0ebafe1d39fa3e9b8a1a9c7461e6e9"
        );
        assert!(source.retained_objects().is_empty());
        assert_eq!(source.retained_git_segments(), vec![&pack.segment]);
    }

    #[test]
    fn accepted_git_head_rejects_spans_that_do_not_reach_its_sequence() {
        let head_oid = "a".repeat(40);
        let source = RunSource::accepted_git_head(
            "owner/repo",
            GitHead::new(head_oid.clone(), 2, 7),
            vec![GitPackSpan {
                first_sequence: 1,
                last_sequence: 1,
                geometric_tier: 0,
                base_oid: None,
                head_oid: head_oid.clone(),
                segment: git_segment('c'),
            }],
            ProjectionViewKey::Private,
        );

        assert!(source.unwrap_err().message.contains("do not reach"));
    }

    fn git_segment(character: char) -> GitSegmentRef {
        GitSegmentRef {
            segment_id: format!("segment-{character}"),
            sha256: character.to_string().repeat(64),
            plaintext_bytes: 1,
            encoding_version: 2,
        }
    }
}
