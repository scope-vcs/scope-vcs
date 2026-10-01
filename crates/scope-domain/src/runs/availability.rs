//! Who may use Scope's hosted runner. A repository may create and run native
//! runs only while an operator lists its owning account.

use super::validation::required;
use crate::error::DomainError;

/// Shown wherever a repository's native runs are refused or withdrawn.
pub const NATIVE_RUNS_UNAVAILABLE: &str = "Scope runs are not available for this repository.";

/// The longest note an operator may keep with a listed account.
pub const MAX_NATIVE_RUNS_NOTE_CHARS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeRunsAvailability {
    Available,
    Unavailable,
}

impl NativeRunsAvailability {
    /// `owner_listed` is whether the repository's owning account is on the list.
    pub fn for_owner(owner_listed: bool) -> Self {
        if owner_listed {
            Self::Available
        } else {
            Self::Unavailable
        }
    }

    pub fn is_available(self) -> bool {
        self == Self::Available
    }

    /// Creating, retrying, or admitting a native run requires an available repository.
    pub fn require(self) -> Result<(), DomainError> {
        if self.is_available() {
            Ok(())
        } else {
            Err(DomainError::forbidden(NATIVE_RUNS_UNAVAILABLE))
        }
    }
}

/// An account an operator listed for native runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeRunsAccount {
    pub user_id: String,
    pub added_at_unix: u64,
    pub note: Option<String>,
}

impl NativeRunsAccount {
    pub fn new(
        user_id: impl Into<String>,
        note: Option<String>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        let note = note
            .map(|note| note.trim().to_string())
            .filter(|note| !note.is_empty());
        if note
            .as_ref()
            .is_some_and(|note| note.chars().count() > MAX_NATIVE_RUNS_NOTE_CHARS)
        {
            return Err(DomainError::invalid_input(format!(
                "native runs note must be at most {MAX_NATIVE_RUNS_NOTE_CHARS} characters"
            )));
        }
        Ok(Self {
            user_id: required("native runs account", user_id.into())?,
            added_at_unix: now_unix,
            note,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DomainErrorKind;

    #[test]
    fn only_a_listed_owner_makes_runs_available() {
        let available = NativeRunsAvailability::for_owner(true);
        assert!(available.is_available());
        available.require().unwrap();

        let unavailable = NativeRunsAvailability::for_owner(false);
        assert!(!unavailable.is_available());
        let error = unavailable.require().unwrap_err();
        assert_eq!(error.kind, DomainErrorKind::Forbidden);
        assert_eq!(error.message, NATIVE_RUNS_UNAVAILABLE);
    }

    #[test]
    fn listing_keeps_a_trimmed_bounded_note() {
        let account =
            NativeRunsAccount::new("user_1", Some("  design partner ".into()), 7).unwrap();
        assert_eq!(account.note.as_deref(), Some("design partner"));
        assert_eq!(account.added_at_unix, 7);
        assert_eq!(
            NativeRunsAccount::new("user_1", Some("   ".into()), 7)
                .unwrap()
                .note,
            None
        );
        assert!(
            NativeRunsAccount::new(
                "user_1",
                Some("x".repeat(MAX_NATIVE_RUNS_NOTE_CHARS + 1)),
                7
            )
            .is_err()
        );
        assert!(NativeRunsAccount::new(" ", None, 7).is_err());
    }
}
