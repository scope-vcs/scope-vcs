use crate::{
    policy::ScopePath,
    repo_config::RepoConfig,
    repo_control::is_request_protected_path,
    views::{ViewId, Views},
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestViewPathError {
    ProtectedPath,
    HiddenPath,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathHistory {
    pub live_paths: BTreeSet<ScopePath>,
    pub file_change_labels: Vec<(ScopePath, ViewId)>,
    pub visibility_changes: Vec<(ScopePath, ViewId, ViewId)>,
}

pub struct RequestViewPaths<'a> {
    repo_config: &'a RepoConfig,
    views: &'a Views,
    view: &'a ViewId,
    labels: BTreeSet<ViewId>,
    visible_paths: &'a BTreeSet<String>,
    live_paths: &'a BTreeSet<ScopePath>,
    hidden_history_paths: BTreeSet<&'a ScopePath>,
}

impl<'a> RequestViewPaths<'a> {
    pub fn new(
        repo_config: &'a RepoConfig,
        views: &'a Views,
        view: &'a ViewId,
        visible_paths: &'a BTreeSet<String>,
        history: &'a PathHistory,
    ) -> Self {
        let labels = views.labels(view);
        let hidden_history_paths = history
            .file_change_labels
            .iter()
            .filter(|(_, label)| !labels.contains(label))
            .map(|(path, _)| path)
            .chain(
                history
                    .visibility_changes
                    .iter()
                    .filter(|(_, old_label, new_label)| {
                        !labels.contains(old_label) || !labels.contains(new_label)
                    })
                    .map(|(path, _, _)| path),
            )
            .collect();
        Self {
            repo_config,
            views,
            view,
            labels,
            visible_paths,
            live_paths: &history.live_paths,
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
        if self.live_paths.contains(path) || self.hidden_history_paths.contains(path) {
            return Err(RequestViewPathError::HiddenPath);
        }
        if self.labels.contains(&self.repo_config.label_for_path(path)) {
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
