use crate::{
    policy::ScopePath, repo_config::RepoConfig, repo_control::is_public_request_protected_path,
    views::ViewId,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicRequestPathError {
    ProtectedPath,
    PrivatePath,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathHistory {
    pub live_paths: BTreeSet<ScopePath>,
    pub file_change_labels: Vec<(ScopePath, ViewId)>,
    pub visibility_changes: Vec<(ScopePath, ViewId, ViewId)>,
}

pub struct PublicRequestPaths<'a> {
    repo_config: &'a RepoConfig,
    public_visible_paths: &'a BTreeSet<String>,
    live_paths: &'a BTreeSet<ScopePath>,
    private_history_paths: BTreeSet<&'a ScopePath>,
}

impl<'a> PublicRequestPaths<'a> {
    pub fn new(
        repo_config: &'a RepoConfig,
        public_visible_paths: &'a BTreeSet<String>,
        history: &'a PathHistory,
    ) -> Self {
        let private_history_paths = history
            .file_change_labels
            .iter()
            .filter(|(_, label)| label.is_private())
            .map(|(path, _)| path)
            .chain(
                history
                    .visibility_changes
                    .iter()
                    .filter(|(_, old_label, new_label)| {
                        old_label.is_private() || new_label.is_private()
                    })
                    .map(|(path, _, _)| path),
            )
            .collect();
        Self {
            repo_config,
            public_visible_paths,
            live_paths: &history.live_paths,
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
        if self.live_paths.contains(path) || self.private_history_paths.contains(path) {
            return Err(PublicRequestPathError::PrivatePath);
        }
        if self.repo_config.label_for_path(path).is_public() {
            Ok(())
        } else {
            Err(PublicRequestPathError::PrivatePath)
        }
    }
}

#[cfg(test)]
mod tests;
