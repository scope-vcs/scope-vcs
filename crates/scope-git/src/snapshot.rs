use crate::GitStorageLimitError;
use scope_domain::repository::git::{GitHead, GitPoolPackOwner};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedGitPush {
    pub head: GitHead,
    pub owner: GitPoolPackOwner,
    pub prerequisites: Vec<String>,
}

pub fn prepare_git_push(
    head_oid: String,
    previous: Option<&GitHead>,
) -> Result<PreparedGitPush, GitStorageLimitError> {
    let sequence = next_push_sequence(previous.map(|head| head.push_sequence))?;
    Ok(PreparedGitPush {
        head: GitHead::new(
            head_oid,
            sequence,
            previous.map_or(1, |head| head.change_version.saturating_add(1)),
        ),
        owner: GitPoolPackOwner::Main {
            first_sequence: sequence,
            last_sequence: sequence,
        },
        prerequisites: previous
            .map(|head| vec![head.head_oid.clone()])
            .unwrap_or_default(),
    })
}

fn next_push_sequence(previous_sequence: Option<u64>) -> Result<u64, GitStorageLimitError> {
    previous_sequence
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(GitStorageLimitError::SequenceOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_push_packs_the_next_sequence_on_top_of_the_previous_head() {
        let previous = GitHead::new("head-1".to_string(), 4, 9);

        let prepared = prepare_git_push("head-2".to_string(), Some(&previous)).unwrap();

        assert_eq!(prepared.head, GitHead::new("head-2".to_string(), 5, 10));
        assert_eq!(
            prepared.owner,
            GitPoolPackOwner::Main {
                first_sequence: 5,
                last_sequence: 5,
            }
        );
        assert_eq!(prepared.prerequisites, ["head-1"]);

        let first = prepare_git_push("head-1".to_string(), None).unwrap();

        assert_eq!(first.head, GitHead::new("head-1".to_string(), 1, 1));
        assert_eq!(
            first.owner,
            GitPoolPackOwner::Main {
                first_sequence: 1,
                last_sequence: 1,
            }
        );
        assert!(first.prerequisites.is_empty());
    }

    #[test]
    fn preparation_rejects_a_sequence_overflow() {
        let previous = GitHead::new("head".to_string(), u64::MAX, 1);

        let error = prepare_git_push("next".to_string(), Some(&previous)).unwrap_err();

        assert_eq!(error.to_string(), "Git push sequence overflow");
    }
}
