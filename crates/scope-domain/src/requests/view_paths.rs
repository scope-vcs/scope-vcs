use crate::{
    policy::ScopePath,
    repo_control::is_request_protected_path,
    repository::Repository,
    views::{ViewId, Views},
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestViewPathError {
    ProtectedPath,
    HiddenPath,
}

pub struct RequestViewPaths<'a> {
    repository: &'a Repository,
    views: &'a Views,
    view: &'a ViewId,
    labels: BTreeSet<ViewId>,
    visible_paths: &'a BTreeSet<String>,
    hidden_history_paths: BTreeSet<&'a ScopePath>,
}

impl<'a> RequestViewPaths<'a> {
    pub fn new(
        repository: &'a Repository,
        views: &'a Views,
        view: &'a ViewId,
        visible_paths: &'a BTreeSet<String>,
    ) -> Self {
        let labels = views.labels(view);
        let hidden_history_paths = repository
            .graph
            .commits
            .iter()
            .flat_map(|commit| &commit.changes)
            .filter(|change| !labels.contains(&change.label))
            .map(|change| &change.path)
            .chain(
                repository
                    .visibility_change_sets
                    .iter()
                    .flat_map(|set| &set.changes)
                    .filter(|change| {
                        !labels.contains(&change.old_label) || !labels.contains(&change.new_label)
                    })
                    .map(|change| &change.path),
            )
            .collect();
        Self {
            repository,
            views,
            view,
            labels,
            visible_paths,
            hidden_history_paths,
        }
    }

    pub fn ensure_editable(&self, path: &ScopePath) -> Result<(), RequestViewPathError> {
        if is_request_protected_path(path) {
            return Err(RequestViewPathError::ProtectedPath);
        }
        if self.visible_paths.contains(path.as_str()) {
            return Ok(());
        }
        if self.repository.live_file_exists(path) || self.hidden_history_paths.contains(path) {
            return Err(RequestViewPathError::HiddenPath);
        }
        if self
            .labels
            .contains(&self.repository.repo_config.label_for_path(path))
        {
            Ok(())
        } else {
            Err(RequestViewPathError::HiddenPath)
        }
    }

    pub fn rejection(&self, path: &ScopePath, error: RequestViewPathError) -> String {
        let view = self.views.display_name(self.view);
        match error {
            RequestViewPathError::ProtectedPath => format!(
                "{view} requests cannot change maintainer-controlled paths: {}",
                path.as_str()
            ),
            RequestViewPathError::HiddenPath => {
                format!("{} is not shown by the {view} view", path.as_str())
            }
        }
    }
}

#[cfg(test)]
mod tests;
