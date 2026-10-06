use crate::views::ViewId;
use crate::{
    policy::ScopePath, repo_control::is_public_request_protected_path, repository::Repository,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicRequestPathError {
    ProtectedPath,
    PrivatePath,
}

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
            .filter(|change| !change.label.is_public())
            .map(|change| &change.path)
            .chain(
                repository
                    .visibility_change_sets
                    .iter()
                    .flat_map(|set| &set.changes)
                    .filter(|change| !change.old_label.is_public() || !change.new_label.is_public())
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
        if self.repository.live_file_exists(path) || self.private_history_paths.contains(path) {
            return Err(PublicRequestPathError::PrivatePath);
        }
        if self.repository.repo_config.label_for_path(path) == ViewId::public() {
            Ok(())
        } else {
            Err(PublicRequestPathError::PrivatePath)
        }
    }
}

#[cfg(test)]
mod tests;
