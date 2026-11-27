use super::error::{ReviewedUpdateError, ReviewedUpdateResult};
use crate::{
    policy::{LabelRule, Policy, ScopePath},
    repo_config::RepoConfig,
};

pub(super) fn policy_from_config_for_tree<'a>(
    config: &RepoConfig,
    paths: impl IntoIterator<Item = &'a ScopePath>,
) -> ReviewedUpdateResult<Policy> {
    let mut policy = Policy::new(config.files.default_view());
    policy
        .add_rules(paths.into_iter().map(|path| LabelRule {
            path: path.clone(),
            view: config.label_for_path(path),
        }))
        .map_err(ReviewedUpdateError::InvalidPolicy)?;
    Ok(policy)
}

pub fn policy_matches_config<'a>(
    policy: &Policy,
    config: &RepoConfig,
    paths: impl IntoIterator<Item = &'a ScopePath>,
) -> bool {
    paths
        .into_iter()
        .all(|path| policy.label(path, config.views()) == config.label_for_path(path))
}
