use super::validation::{validate_git_oid, validate_sha256_hash};
use crate::{
    content::SourceBlob, content_ref::ContentRef, error::DomainError, repository::git::GitHead,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunTrigger {
    Manual,
    PushMain,
    Request,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RunSource {
    EphemeralGitBundle {
        object: SourceBlob,
    },
    RequestGitSnapshot {
        object: SourceBlob,
        base_oid: String,
    },
    AcceptedGitHead {
        repository_id: String,
        head: GitHead,
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

    pub fn request_git_snapshot(object: SourceBlob, base_oid: String) -> Result<Self, DomainError> {
        validate_source_blob(&object, "request run source snapshot")?;
        if !matches!(object.content_ref, ContentRef::GitBundleSha256(_)) {
            return Err(DomainError::invalid_input(
                "request run source snapshot must be a Git bundle",
            ));
        }
        validate_git_oid("request run source base", &base_oid)?;
        Ok(Self::RequestGitSnapshot { object, base_oid })
    }

    pub fn accepted_git_head(
        repository_id: impl Into<String>,
        head: GitHead,
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
        Ok(Self::AcceptedGitHead {
            repository_id,
            head,
        })
    }

    pub fn ephemeral_bundle(&self) -> Option<&SourceBlob> {
        match self {
            Self::EphemeralGitBundle { object } => Some(object),
            Self::RequestGitSnapshot { .. } | Self::AcceptedGitHead { .. } => None,
        }
    }

    pub fn request_git_source(&self) -> Option<(&SourceBlob, &str)> {
        match self {
            Self::RequestGitSnapshot { object, base_oid } => Some((object, base_oid)),
            Self::EphemeralGitBundle { .. } | Self::AcceptedGitHead { .. } => None,
        }
    }

    pub fn retained_objects(&self) -> Vec<&SourceBlob> {
        match self {
            Self::EphemeralGitBundle { object } | Self::RequestGitSnapshot { object, .. } => {
                vec![object]
            }
            Self::AcceptedGitHead { .. } => Vec::new(),
        }
    }

    pub fn source_identity(&self) -> &str {
        match self {
            Self::EphemeralGitBundle { object } | Self::RequestGitSnapshot { object, .. } => {
                &object.sha256
            }
            Self::AcceptedGitHead { head, .. } => head.frontier.digest(),
        }
    }

    pub fn git_oid(&self) -> &str {
        match self {
            Self::EphemeralGitBundle { object } | Self::RequestGitSnapshot { object, .. } => {
                &object.git_oid
            }
            Self::AcceptedGitHead { head, .. } => &head.head_oid,
        }
    }

    pub fn logical_git_head(&self) -> Option<(&str, &GitHead)> {
        match self {
            Self::AcceptedGitHead {
                repository_id,
                head,
            } => Some((repository_id, head)),
            Self::EphemeralGitBundle { .. } | Self::RequestGitSnapshot { .. } => None,
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
    fn accepted_git_head_pins_only_the_head() {
        let head_oid = "a".repeat(40);
        let source =
            RunSource::accepted_git_head("owner/repo", GitHead::new(head_oid.clone(), 1, 7))
                .unwrap();

        assert_eq!(source.git_oid(), head_oid);
        assert_eq!(
            source.source_identity(),
            "189efc3e1b3dd4adb49c858cdb3be61d3a0ebafe1d39fa3e9b8a1a9c7461e6e9"
        );
        assert!(source.retained_objects().is_empty());
        assert_eq!(
            serde_json::to_value(&source).unwrap(),
            serde_json::json!({
                "kind": "accepted-git-head",
                "repository_id": "owner/repo",
                "head": {
                    "head_oid": head_oid,
                    "push_sequence": 1,
                    "change_version": 7,
                    "frontier": "189efc3e1b3dd4adb49c858cdb3be61d3a0ebafe1d39fa3e9b8a1a9c7461e6e9",
                },
            })
        );
    }

    #[test]
    fn accepted_git_head_rejects_an_unsequenced_head() {
        let source = RunSource::accepted_git_head("owner/repo", GitHead::new("a".repeat(40), 0, 7));

        assert!(source.unwrap_err().message.contains("must be positive"));
    }
}
