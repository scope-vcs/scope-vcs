use crate::{
    policy::{ScopePath, Visibility},
    repo_control::is_public_request_protected_path,
    repository::Repository,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicRequestPathError {
    ProtectedPath,
    PrivatePath,
}

/// Repository facts shared by every path/commit in one public request validation.
/// Current public paths may be edited even if they were private in older history.
pub struct PublicRequestPaths<'a> {
    repository: &'a Repository,
    public_visible_paths: &'a BTreeSet<String>,
    private_history_paths: BTreeSet<&'a ScopePath>,
}

impl<'a> PublicRequestPaths<'a> {
    pub fn new(repository: &'a Repository, public_visible_paths: &'a BTreeSet<String>) -> Self {
        let private_history_paths = repository
            .graph
            .commits
            .iter()
            .flat_map(|commit| &commit.changes)
            .filter(|change| change.visibility == Visibility::Private)
            .map(|change| &change.path)
            .chain(
                repository
                    .visibility_change_sets
                    .iter()
                    .flat_map(|set| &set.changes)
                    .filter(|change| {
                        change.old_visibility == Visibility::Private
                            || change.new_visibility == Visibility::Private
                    })
                    .map(|change| &change.path),
            )
            .collect();
        Self {
            repository,
            public_visible_paths,
            private_history_paths,
        }
    }

    pub fn ensure_editable(&self, path: &ScopePath) -> Result<(), PublicRequestPathError> {
        if is_public_request_protected_path(path) {
            return Err(PublicRequestPathError::ProtectedPath);
        }
        if self.public_visible_paths.contains(path.as_str()) {
            return Ok(());
        }
        if self.repository.graph_has_file(path) || self.private_history_paths.contains(path) {
            return Err(PublicRequestPathError::PrivatePath);
        }
        if self.repository.repo_config.visibility_for_path(path) == Visibility::Public {
            Ok(())
        } else {
            Err(PublicRequestPathError::PrivatePath)
        }
    }
}

#[cfg(test)]
mod tests;
