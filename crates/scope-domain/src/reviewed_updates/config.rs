use super::{
    error::{ReviewedUpdateError, ReviewedUpdateResult},
    history_rewrite::{HistoryRewriteInput, apply_history_rewrites},
    policy::policy_from_config_for_tree,
    views::views_transition,
};
use crate::{
    repo_config::RepoConfig,
    repository::Repository,
    views::ViewId,
    visibility_changes::{VisibilityChange, VisibilityChangeSet, visibility_change_set_id},
};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct ReviewedConfigUpdateInput {
    pub occurred_at_unix: i64,
    pub author_id: String,
    pub config: RepoConfig,
}

pub fn apply_reviewed_config_to_repo(
    repo: &mut Repository,
    update: ReviewedConfigUpdateInput,
) -> ReviewedUpdateResult<bool> {
    if repo.repo_config == update.config {
        return Ok(false);
    }
    let views = views_transition(repo, &update.config).map_err(ReviewedUpdateError::Domain)?;
    let live_tree = repo.live_files.clone();
    let after_commit_id = repo.graph.commits.last().map(|commit| commit.id.clone());
    let history_rewrites = update
        .config
        .history_rewrites_added_since(Some(&repo.repo_config));
    let history_rewrite = apply_history_rewrites(
        repo,
        HistoryRewriteInput {
            config: &update.config,
            rewrites: &history_rewrites,
            live_tree: &live_tree,
            changed_paths: &BTreeSet::new(),
        },
    );

    let mut visibility_changes = history_rewrite.visibility_changes;
    let baseline_paths = visibility_changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<BTreeSet<_>>();
    for (path, current_content) in &live_tree {
        let old_label = repo.policy.label(path, repo.repo_config.views());
        let new_label = update.config.label_for_path(path);
        if old_label == new_label || baseline_paths.contains(path) {
            continue;
        }
        if history_rewrite.redacted_paths.contains(path)
            && old_label == ViewId::public()
            && new_label == ViewId::private()
        {
            continue;
        }

        visibility_changes.push(VisibilityChange {
            path: path.clone(),
            old_label,
            new_label,
            current_content: Some(current_content.clone()),
        });
    }

    repo.policy = policy_from_config_for_tree(&update.config, live_tree.keys())?;
    repo.repo_config = update.config;
    if !visibility_changes.is_empty() || views.is_some() {
        let mut set = VisibilityChangeSet::new(
            visibility_change_set_id(repo.record.change_version.saturating_add(1)),
            after_commit_id,
            None,
            update.author_id,
            visibility_changes,
            views,
        )
        .map_err(ReviewedUpdateError::Conflict)?;
        set.occurred_at_unix = Some(update.occurred_at_unix);
        repo.visibility_change_sets.push(set);
    }
    repo.bump_content_version();
    Ok(true)
}
