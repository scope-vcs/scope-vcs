use super::{
    error::{ReviewedUpdateError, ReviewedUpdateResult},
    history_rewrite::{HistoryRewriteInput, apply_history_rewrites},
    policy::policy_from_config_for_tree,
    views::views_transition,
};
use crate::views::ViewId;
use crate::{
    content::SourceBlob,
    error::DomainError,
    policy::{LabelRule, Policy, ScopePath},
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin},
    repo_config::RepoConfig,
    repo_control::is_request_protected_path,
    repository::{
        RepoLifecycleState, Repository,
        access::{MainPushMode, RepositoryAccess, RepositoryActor},
        git::{GitHead, GitPackSpan},
        updates::RequestMergeOrigin,
    },
    visibility_changes::{VisibilityChange, VisibilityChangeSet, visibility_change_set_id},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct ReviewedContentChange {
    pub path: ScopePath,
    pub content: Option<SourceBlob>,
}

#[derive(Clone, Debug)]
pub struct ReviewedUpdateInput {
    pub occurred_at_unix: Option<i64>,
    pub branch: String,
    pub author_id: String,
    pub message: String,
    pub git_head: GitHead,
    pub git_pack_span: GitPackSpan,
    pub changes: Vec<ReviewedContentChange>,
    pub previous_config: Option<RepoConfig>,
    pub config: RepoConfig,
    pub open_requests_by_view: BTreeMap<ViewId, usize>,
}

#[derive(Clone, Debug)]
pub struct ReviewedUpdateAuthorization<'a> {
    pub access: RepositoryAccess,
    pub push_mode: MainPushMode,
    pub current_config: &'a RepoConfig,
    pub proposed_config: &'a RepoConfig,
}

pub fn authorize_reviewed_update(
    authorization: ReviewedUpdateAuthorization<'_>,
) -> Result<(), DomainError> {
    match &authorization.push_mode {
        MainPushMode::Denied => {
            let message = if authorization.access.actor == RepositoryActor::Public {
                "repo membership required"
            } else {
                "push permission required"
            };
            return Err(DomainError::forbidden(message));
        }
        MainPushMode::ThroughView(view) => {
            return Err(DomainError::forbidden(format!(
                "pushes to main through the {view} view land as requests"
            )));
        }
        MainPushMode::FirstPush | MainPushMode::Ready => {}
    }
    if !authorization.access.can_change_file_visibility
        && authorization.current_config != authorization.proposed_config
    {
        return Err(DomainError::forbidden(
            "file visibility permission required",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct ContentPushState {
    pub change_version: u64,
    pub content_version: u64,
    pub policy: Policy,
    pub repo_config: RepoConfig,
    pub live_files: BTreeMap<ScopePath, SourceBlob>,
    pub git_head: Option<GitHead>,
}

#[derive(Clone, Debug)]
pub struct AcceptedContentPush {
    pub change_version: u64,
    pub content_version: u64,
    pub policy: Policy,
    pub git_head: GitHead,
    pub git_pack_span: GitPackSpan,
    pub logical_commit: LogicalCommit,
}

pub fn source_content_matches(left: Option<&SourceBlob>, right: Option<&SourceBlob>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.sha256 == right.sha256
                && left.git_oid == right.git_oid
                && left.git_file_mode == right.git_file_mode
                && left.size_bytes == right.size_bytes
        }
        (None, None) => true,
        _ => false,
    }
}

pub fn apply_reviewed_update_to_repo(
    repo: &mut Repository,
    update: ReviewedUpdateInput,
) -> ReviewedUpdateResult<()> {
    validate_git_push_transition(
        repo.git_head.as_ref(),
        &update.git_head,
        &update.git_pack_span,
    )?;
    if update.changes.is_empty() {
        return Err(ReviewedUpdateError::BadRequest(
            "update must include file changes",
        ));
    }
    if update.config == repo.repo_config
        && update
            .previous_config
            .as_ref()
            .is_some_and(|previous| previous == &repo.repo_config)
    {
        return apply_content_only_update(repo, update);
    }
    let views = views_transition(repo, &update.config, &update.open_requests_by_view)
        .map_err(ReviewedUpdateError::Domain)?;
    let old_tree = repo.live_files.clone();
    let mut file_changes = build_file_changes(
        &old_tree,
        &repo.policy,
        &update.config,
        update.changes,
        WrittenFileVisibility::FromConfig,
    );
    let mut new_tree = old_tree.clone();
    for change in &file_changes {
        match &change.new_content {
            Some(content) => {
                new_tree.insert(change.path.clone(), content.clone());
            }
            None => {
                new_tree.remove(&change.path);
            }
        }
    }

    if file_changes.is_empty() {
        return Err(ReviewedUpdateError::BadRequest(
            "update did not change the live tree",
        ));
    }

    let changed_paths = file_changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<BTreeSet<_>>();
    let logical_id = format!("rv_push_{}", update.git_head.head_oid);
    let after_commit_id = repo.graph.commits.last().map(|commit| commit.id.clone());
    let history_rewrites = update
        .config
        .history_rewrites_added_since(update.previous_config.as_ref());
    let history_rewrite = apply_history_rewrites(
        repo,
        HistoryRewriteInput {
            config: &update.config,
            rewrites: &history_rewrites,
            live_tree: &old_tree,
            changed_paths: &changed_paths,
        },
    );
    for change in &mut file_changes {
        if change.new_content.is_none() && history_rewrite.redacted_paths.contains(&change.path) {
            change.label = ViewId::private();
        }
    }
    let mut visibility_changes = history_rewrite.visibility_changes;
    let baseline_paths = visibility_changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<BTreeSet<_>>();
    for (path, current_content) in &new_tree {
        let old_label = repo.policy.label(path, repo.repo_config.views());
        let new_label = update.config.label_for_path(path);
        if old_label == new_label {
            continue;
        }
        if baseline_paths.contains(path) {
            continue;
        }
        if history_rewrite.redacted_paths.contains(path)
            && old_label == ViewId::public()
            && new_label == ViewId::private()
        {
            continue;
        }
        if old_label == ViewId::public()
            && new_label == ViewId::private()
            && !old_tree.contains_key(path)
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

    let next_policy = policy_from_config_for_tree(&update.config, new_tree.keys())?;
    let next_config = update.config.clone();

    if !visibility_changes.is_empty() || views.is_some() {
        let mut set = VisibilityChangeSet::new(
            visibility_change_set_id(repo.record.change_version.saturating_add(1)),
            after_commit_id,
            Some(logical_id.clone()),
            update.author_id.clone(),
            visibility_changes,
            views,
        )
        .map_err(ReviewedUpdateError::Conflict)?;
        set.occurred_at_unix = update.occurred_at_unix;
        repo.visibility_change_sets.push(set);
    }

    repo.graph.commits.push(LogicalCommit {
        occurred_at_unix: update.occurred_at_unix,
        id: logical_id,
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: update.git_head.head_oid.clone(),
        },
        author_id: update.author_id,
        message: update.message,
        changes: file_changes,
    });
    repo.live_files = new_tree;
    repo.policy = next_policy;
    repo.repo_config = next_config;
    repo.git_pack_spans.push(update.git_pack_span);
    repo.git_head = Some(update.git_head);
    repo.first_push_token = None;
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    repo.bump_content_version();
    Ok(())
}

fn apply_content_only_update(
    repo: &mut Repository,
    update: ReviewedUpdateInput,
) -> ReviewedUpdateResult<()> {
    let accepted = accept_content_push(
        ContentPushState {
            change_version: repo.record.change_version,
            content_version: repo.record.content_version,
            policy: repo.policy.clone(),
            repo_config: repo.repo_config.clone(),
            live_files: repo.live_files.clone(),
            git_head: repo.git_head.clone(),
        },
        update,
    )?;
    apply_accepted_content_push(repo, accepted);
    Ok(())
}

pub fn apply_request_merge_to_repo(
    repo: &mut Repository,
    update: ReviewedUpdateInput,
    origin: RequestMergeOrigin,
) -> ReviewedUpdateResult<()> {
    let accepted = accept_request_merge(
        ContentPushState {
            change_version: repo.record.change_version,
            content_version: repo.record.content_version,
            policy: repo.policy.clone(),
            repo_config: repo.repo_config.clone(),
            live_files: repo.live_files.clone(),
            git_head: repo.git_head.clone(),
        },
        update,
        origin,
    )?;
    apply_accepted_content_push(repo, accepted);
    Ok(())
}

fn apply_accepted_content_push(repo: &mut Repository, accepted: AcceptedContentPush) {
    for change in &accepted.logical_commit.changes {
        match &change.new_content {
            Some(content) => {
                repo.live_files.insert(change.path.clone(), content.clone());
            }
            None => {
                repo.live_files.remove(&change.path);
            }
        }
    }
    repo.record.change_version = accepted.change_version;
    repo.record.content_version = accepted.content_version;
    repo.policy = accepted.policy;
    repo.graph.commits.push(accepted.logical_commit);
    repo.git_pack_spans.push(accepted.git_pack_span);
    repo.git_head = Some(accepted.git_head);
    repo.first_push_token = None;
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
}

pub fn accept_content_push(
    state: ContentPushState,
    update: ReviewedUpdateInput,
) -> ReviewedUpdateResult<AcceptedContentPush> {
    let source_head_oid = update.git_head.head_oid.clone();
    accept_content_update(
        state,
        update,
        false,
        LogicalCommitOrigin::CanonicalPush { source_head_oid },
    )
}

pub fn accept_request_merge(
    state: ContentPushState,
    update: ReviewedUpdateInput,
    origin: RequestMergeOrigin,
) -> ReviewedUpdateResult<AcceptedContentPush> {
    accept_content_update(state, update, true, origin.into_logical_origin())
}

fn accept_content_update(
    state: ContentPushState,
    mut update: ReviewedUpdateInput,
    allow_unchanged_tree: bool,
    origin: LogicalCommitOrigin,
) -> ReviewedUpdateResult<AcceptedContentPush> {
    validate_git_push_transition(
        state.git_head.as_ref(),
        &update.git_head,
        &update.git_pack_span,
    )?;
    if update.changes.is_empty() && !allow_unchanged_tree {
        return Err(ReviewedUpdateError::BadRequest(
            "update must include file changes",
        ));
    }
    if update.config != state.repo_config {
        return Err(ReviewedUpdateError::Conflict(
            "repo config changed since review; rerun scope push --main",
        ));
    }
    let file_changes = build_file_changes(
        &state.live_files,
        &state.policy,
        &update.config,
        update.changes,
        WrittenFileVisibility::ExistingFromPolicy,
    );
    if file_changes.is_empty() && !allow_unchanged_tree {
        return Err(ReviewedUpdateError::BadRequest(
            "update did not change the live tree",
        ));
    }
    validate_commit_origin(&origin, &file_changes, &update.config)?;
    let mut policy = state.policy;
    for change in &file_changes {
        if change.old_content.is_some() && change.new_content.is_none() {
            policy.remove_rule(&change.path);
        }
    }
    policy
        .add_rules(
            file_changes
                .iter()
                .filter(|change| change.old_content.is_none() && change.new_content.is_some())
                .map(|change| LabelRule {
                    path: change.path.clone(),
                    view: update.config.label_for_path(&change.path),
                }),
        )
        .map_err(ReviewedUpdateError::InvalidPolicy)?;
    let change_version = state.change_version.saturating_add(1);
    let content_version = state.content_version.saturating_add(1);
    update.git_head.change_version = change_version;
    let logical_prefix = if allow_unchanged_tree {
        "rv_merge"
    } else {
        "rv_push"
    };
    let logical_id = format!("{logical_prefix}_{}", update.git_head.head_oid);
    let logical_commit = LogicalCommit {
        occurred_at_unix: update.occurred_at_unix,
        id: logical_id,
        origin,
        author_id: update.author_id,
        message: update.message,
        changes: file_changes,
    };
    Ok(AcceptedContentPush {
        change_version,
        content_version,
        policy,
        git_head: update.git_head,
        git_pack_span: update.git_pack_span,
        logical_commit,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WrittenFileVisibility {
    FromConfig,
    ExistingFromPolicy,
}

fn build_file_changes(
    live_tree: &BTreeMap<ScopePath, SourceBlob>,
    policy: &Policy,
    config: &RepoConfig,
    changes: Vec<ReviewedContentChange>,
    written_visibility: WrittenFileVisibility,
) -> Vec<FileChange> {
    let mut file_changes = Vec::with_capacity(changes.len());
    for change in changes {
        let old_content = live_tree.get(&change.path).cloned();
        if source_content_matches(old_content.as_ref(), change.content.as_ref()) {
            continue;
        }
        let visibility = match (written_visibility, &old_content, &change.content) {
            (_, _, None) => policy.label(&change.path, config.views()),
            (WrittenFileVisibility::FromConfig, _, Some(_))
            | (WrittenFileVisibility::ExistingFromPolicy, None, Some(_)) => {
                config.label_for_path(&change.path)
            }
            (WrittenFileVisibility::ExistingFromPolicy, Some(_), Some(_)) => {
                policy.label(&change.path, config.views())
            }
        };
        file_changes.push(FileChange {
            label: visibility,
            path: change.path,
            old_content,
            new_content: change.content,
        });
    }
    file_changes
}

fn validate_git_push_transition(
    previous: Option<&GitHead>,
    next: &GitHead,
    span: &GitPackSpan,
) -> ReviewedUpdateResult<()> {
    if span.first_sequence != next.push_sequence
        || span.last_sequence != next.push_sequence
        || span.geometric_tier != 0
        || span.head_oid != next.head_oid
    {
        return Err(ReviewedUpdateError::Conflict(
            "Git push pack span does not match the logical head",
        ));
    }
    let expected_sequence = previous
        .map_or(Some(1), |head| head.push_sequence.checked_add(1))
        .ok_or(ReviewedUpdateError::Conflict("Git push sequence overflow"))?;
    let expected_base = previous.map(|head| head.head_oid.as_str());
    if next.push_sequence != expected_sequence || span.base_oid.as_deref() != expected_base {
        return Err(ReviewedUpdateError::Conflict(
            "Git push does not advance the current pack frontier",
        ));
    }
    Ok(())
}

fn validate_commit_origin(
    origin: &LogicalCommitOrigin,
    changes: &[FileChange],
    repo_config: &RepoConfig,
) -> ReviewedUpdateResult<()> {
    let LogicalCommitOrigin::RequestMerge {
        view,
        base_oid,
        parent_oids,
        request_head_oid,
        commits,
        ..
    } = origin
    else {
        return Ok(());
    };
    let views = repo_config.views();
    if views.get(view).is_none() || view == views.full() {
        return Err(ReviewedUpdateError::Conflict(
            "request merge preserves commits only for a view narrower than the full view",
        ));
    }
    let labels = views.labels(view);
    let editable = |path: &ScopePath, label: &ViewId| {
        labels.contains(label) && !is_request_protected_path(path)
    };

    if changes
        .iter()
        .any(|change| !editable(&change.path, &change.label))
    {
        return Err(ReviewedUpdateError::Conflict(
            "request merge contains changes outside its view",
        ));
    }
    let Some(last) = commits.last() else {
        return Err(ReviewedUpdateError::Conflict(
            "request merge has no native commits",
        ));
    };
    if &last.oid != request_head_oid {
        return Err(ReviewedUpdateError::Conflict(
            "request merge native commits do not end at request head",
        ));
    }
    let range_oids = commits
        .iter()
        .map(|commit| commit.oid.as_str())
        .collect::<BTreeSet<_>>();
    let view_parent_oids = parent_oids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if range_oids.len() != commits.len()
        || view_parent_oids.is_empty()
        || view_parent_oids.len() != parent_oids.len()
        || parent_oids.iter().any(String::is_empty)
        || commits.iter().any(|commit| {
            commit.oid.is_empty()
                || commit.tree_oid.is_empty()
                || commit.parent_oids.is_empty()
                || commit.parent_oids.iter().any(String::is_empty)
        })
    {
        return Err(ReviewedUpdateError::Conflict(
            "request merge contains malformed native commit facts",
        ));
    }
    let touched_paths = commits
        .iter()
        .flat_map(|commit| commit.changed_paths.iter())
        .collect::<BTreeSet<_>>();
    if touched_paths
        .iter()
        .any(|path| !editable(path, &repo_config.label_for_path(path)))
        || changes
            .iter()
            .any(|change| !touched_paths.contains(&change.path))
    {
        return Err(ReviewedUpdateError::Conflict(
            "request merge native paths do not cover the logical changes",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut descends_from_base = false;
    for commit in commits {
        for parent_oid in &commit.parent_oids {
            descends_from_base |= parent_oid == base_oid;
            if range_oids.contains(parent_oid.as_str()) {
                if seen.contains(parent_oid.as_str()) {
                    continue;
                }
                return Err(ReviewedUpdateError::Conflict(
                    "request merge native commits are not ordered ancestor-first",
                ));
            }
            if !view_parent_oids.contains(parent_oid.as_str()) {
                return Err(ReviewedUpdateError::Domain(DomainError::conflict(format!(
                    "request merge contains a parent outside {} history",
                    views.display_name(view)
                ))));
            }
        }
        seen.insert(commit.oid.as_str());
    }
    if !descends_from_base || !view_parent_oids.contains(base_oid.as_str()) {
        return Err(ReviewedUpdateError::Conflict(
            "request merge does not include its view's current base as a parent",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod authorization_tests {
    use super::*;
    use crate::{error::DomainErrorKind, repository::access::RepositoryActor, views::ViewId};

    fn access(
        actor: RepositoryActor,
        can_push: bool,
        can_change_visibility: bool,
    ) -> RepositoryAccess {
        RepositoryAccess {
            actor,
            view: if actor == RepositoryActor::Public {
                ViewId::public()
            } else {
                ViewId::private()
            },
            can_push,
            can_change_file_visibility: can_change_visibility,
            can_manage_members: false,
            can_delete_repo: false,
        }
    }

    #[test]
    fn reviewed_update_authorization_masks_public_push_denial_as_membership_required() {
        let config = RepoConfig::with_default_view(ViewId::private());
        let error = authorize_reviewed_update(ReviewedUpdateAuthorization {
            access: access(RepositoryActor::Public, false, false),
            push_mode: MainPushMode::Denied,
            current_config: &config,
            proposed_config: &config,
        })
        .expect_err("public actor cannot push main");

        assert_eq!(error.kind, DomainErrorKind::Forbidden);
        assert_eq!(error.message, "repo membership required");
    }

    #[test]
    fn reviewed_update_authorization_rechecks_member_push_permission() {
        let config = RepoConfig::with_default_view(ViewId::private());
        let error = authorize_reviewed_update(ReviewedUpdateAuthorization {
            access: access(RepositoryActor::Member, false, false),
            push_mode: MainPushMode::Denied,
            current_config: &config,
            proposed_config: &config,
        })
        .expect_err("member without push permission cannot push main");

        assert_eq!(error.kind, DomainErrorKind::Forbidden);
        assert_eq!(error.message, "push permission required");
    }

    #[test]
    fn reviewed_update_authorization_sends_narrower_pushes_through_requests() {
        let config = RepoConfig::with_default_view(ViewId::private());
        let error = authorize_reviewed_update(ReviewedUpdateAuthorization {
            access: access(RepositoryActor::Member, true, false),
            push_mode: MainPushMode::ThroughView(ViewId::parse("agent").unwrap()),
            current_config: &config,
            proposed_config: &config,
        })
        .expect_err("a narrower member cannot push canonical main directly");

        assert_eq!(error.kind, DomainErrorKind::Forbidden);
        assert_eq!(
            error.message,
            "pushes to main through the agent view land as requests"
        );
    }

    #[test]
    fn reviewed_update_authorization_protects_config_changes() {
        let current = RepoConfig::with_default_view(ViewId::private());
        let proposed = RepoConfig::with_default_view(ViewId::public());
        let error = authorize_reviewed_update(ReviewedUpdateAuthorization {
            access: access(RepositoryActor::Member, true, false),
            push_mode: MainPushMode::Ready,
            current_config: &current,
            proposed_config: &proposed,
        })
        .expect_err("config change requires visibility permission");

        assert_eq!(error.kind, DomainErrorKind::Forbidden);
        assert_eq!(error.message, "file visibility permission required");
    }

    #[test]
    fn reviewed_update_authorization_allows_content_only_push_without_visibility_permission() {
        let config = RepoConfig::with_default_view(ViewId::private());
        authorize_reviewed_update(ReviewedUpdateAuthorization {
            access: access(RepositoryActor::Member, true, false),
            push_mode: MainPushMode::Ready,
            current_config: &config,
            proposed_config: &config,
        })
        .expect("content-only push remains allowed");
    }
}
