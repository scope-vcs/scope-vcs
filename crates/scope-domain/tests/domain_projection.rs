use scope_domain::views::{ViewId, Views};
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    policy::{LabelRule, ScopePath},
    projection::{
        FileChange, LogicalCommit, ProjectionMaterialization, SourceGraph, project_graph,
    },
    projection::{LogicalCommitOrigin, NativeRequestCommit},
    projection_views::{
        ProjectionPreviewCommitVisibility, ProjectionPreviewSource, projection_preview,
    },
    repo_config::{HistoryRewriteAction, HistoryRewriteRequest, RepoConfig, RepoConfigFileRule},
    repository::updates::RequestMergeOrigin,
    repository::{RepoLifecycleState, Repository},
    reviewed_updates::{
        config::{ReviewedConfigUpdateInput, apply_reviewed_config_to_repo},
        content::{
            ContentPushState, ReviewedContentChange, ReviewedUpdateInput, accept_content_push,
            accept_request_merge, apply_reviewed_update_to_repo,
        },
        error::ReviewedUpdateError,
    },
    visibility_changes::{VisibilityChange, VisibilityChangeSet},
};

#[path = "domain_projection/history_metadata.rs"]
mod history_metadata;
#[path = "domain_projection/history_rewrite_baselines.rs"]
mod history_rewrite_baselines;
#[path = "domain_projection/request_views.rs"]
mod request_views;
#[path = "domain_projection/rules.rs"]
mod rules;
#[path = "domain_projection/views.rs"]
mod views;

fn blob(content: &str) -> SourceBlob {
    SourceBlob {
        content_ref: scope_domain::content_ref::ContentRef::blob_sha256(content),
        sha256: format!("sha256:{content}"),
        git_oid: format!("git:{content}"),
        git_file_mode: "100644".to_string(),
        size_bytes: content.len() as u64,
    }
}

fn path(value: &str) -> ScopePath {
    ScopePath::parse(value).unwrap()
}

fn change(
    path_value: &str,
    visibility: ViewId,
    old_content: Option<SourceBlob>,
    new_content: Option<SourceBlob>,
) -> FileChange {
    FileChange {
        label: visibility,
        path: path(path_value),
        old_content,
        new_content,
    }
}

fn added(path_value: &str, visibility: ViewId, content: &str) -> FileChange {
    change(path_value, visibility, None, Some(blob(content)))
}

fn commit(id: &str, message: &str, change: FileChange) -> LogicalCommit {
    LogicalCommit {
        occurred_at_unix: None,
        id: id.to_string(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: id.to_string(),
        },
        author_id: "owner".to_string(),
        message: message.to_string(),
        changes: vec![change],
    }
}

fn graph(commits: Vec<LogicalCommit>) -> SourceGraph {
    SourceGraph {
        repo_id: "scope".to_string(),
        commits,
    }
}

fn visibility_event(
    id: &str,
    after_commit_id: Option<&str>,
    source_commit_id: Option<&str>,
    path_value: &str,
    new_label: ViewId,
    current_content: SourceBlob,
) -> VisibilityChangeSet {
    VisibilityChangeSet {
        occurred_at_unix: None,
        id: id.to_string(),
        anchor_commit_id: after_commit_id.map(str::to_string),
        source_update_id: source_commit_id.map(str::to_string),
        author_id: "owner".to_string(),
        changes: vec![VisibilityChange {
            path: path(path_value),
            old_label: if new_label.is_public() {
                ViewId::private()
            } else {
                ViewId::public()
            },
            new_label,
            current_content: Some(current_content),
        }],
        views: None,
    }
}

type EventSpec<'a> = (&'a str, Option<&'a str>, Option<&'a str>, ViewId, &'a str);

fn project_timeline(
    path_value: &str,
    versions: &[(&str, ViewId, &str)],
    event_specs: &[EventSpec<'_>],
) -> scope_domain::projection::Projection {
    let commits = versions
        .iter()
        .enumerate()
        .map(|(index, (id, visibility, content))| {
            let previous_content = index.checked_sub(1).map(|index| blob(versions[index].2));
            commit(
                id,
                content,
                change(
                    path_value,
                    visibility.clone(),
                    previous_content,
                    Some(blob(content)),
                ),
            )
        })
        .collect();
    let graph = graph(commits);
    let events = event_specs
        .iter()
        .map(|(id, after, source, visibility, content)| {
            visibility_event(
                id,
                *after,
                *source,
                path_value,
                visibility.clone(),
                blob(content),
            )
        })
        .collect::<Vec<_>>();
    project_graph(&graph, &events, &Views::builtin(), &ViewId::public())
}

fn published_test_repo(default_visibility: ViewId) -> Repository {
    let owner = UserAccount {
        id: "owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.com".to_string(),
        email_verified: true,
    };
    let mut repo = Repository::new(&owner, "repo", default_visibility, "repoi_test").unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    repo
}

fn published_repo_with_public_file(message: &str, path: &str, content: &str) -> Repository {
    let mut repo = published_test_repo(ViewId::public());
    let content = blob(content);
    repo.graph.commits.push(commit(
        "rv1",
        message,
        change(path, ViewId::public(), None, Some(content.clone())),
    ));
    repo.live_files.insert(self::path(path), content);
    repo
}

fn config(default: ViewId, rule: Option<(&str, ViewId)>, rewrite_path: Option<&str>) -> RepoConfig {
    let mut config = RepoConfig::with_default_view(default);
    config.files.rules = rule
        .into_iter()
        .map(|(path, visibility)| RepoConfigFileRule {
            path: path.to_string(),
            view: visibility,
        })
        .collect();
    config.history.rewrites = rewrite_path
        .into_iter()
        .map(|path| HistoryRewriteRequest {
            path: path.to_string(),
            action: HistoryRewriteAction::RedactPublicHistory,
        })
        .collect();
    config.validate().unwrap();
    config
}

fn project_repo(repo: &Repository, view_key: ViewId) -> scope_domain::projection::Projection {
    project_graph(
        &repo.graph,
        &repo.visibility_change_sets,
        &Views::builtin(),
        &view_key,
    )
}

#[test]
fn built_in_view_ids_keep_existing_projected_commit_ids() {
    let source = graph(vec![commit(
        "rv1",
        "Initial update",
        added("/README.md", ViewId::public(), "readme"),
    )]);
    for (view, expected) in [
        (ViewId::public(), "pv_public_rv1_1"),
        (ViewId::private(), "pv_private_rv1_1"),
    ] {
        let projection = project_graph(&source, &[], &Views::builtin(), &view);
        assert_eq!(projection.commits[0].projected_id, expected);
    }
}

#[test]
fn preview_commit_labels_are_relative_to_the_requested_view() {
    let mut repo = published_test_repo(ViewId::private());
    let mut mixed = commit(
        "rv1",
        "Mixed update",
        added("/README.md", ViewId::public(), "readme"),
    );
    mixed
        .changes
        .push(added("/secret.md", ViewId::private(), "secret"));
    repo.graph.commits.push(mixed);
    let views = Views::builtin();
    let native = std::collections::BTreeMap::new();
    let source = || ProjectionPreviewSource {
        repo_id: &repo.record.id,
        policy: &repo.policy,
        graph: &repo.graph,
        visibility_change_sets: &repo.visibility_change_sets,
    };
    let public = projection_preview(source(), &views, &ViewId::public(), true, &native).unwrap();
    let private = projection_preview(source(), &views, &ViewId::private(), false, &native).unwrap();
    assert_eq!(
        public.commits[0].visibility,
        ProjectionPreviewCommitVisibility::SomeInView
    );
    assert_eq!(
        private.commits[0].visibility,
        ProjectionPreviewCommitVisibility::AllInView
    );
    assert_eq!(public.summary.hidden_files, 1);
}

fn reviewed_change(path_value: &str, content: Option<&str>) -> ReviewedContentChange {
    ReviewedContentChange {
        path: path(path_value),
        content: content.map(blob),
    }
}

fn apply_update(
    repo: &mut Repository,
    message: &str,
    changes: Vec<ReviewedContentChange>,
    previous_config: Option<RepoConfig>,
    config: RepoConfig,
) {
    apply_update_with_head(
        repo,
        "2222222222222222222222222222222222222222",
        message,
        changes,
        previous_config,
        config,
    );
}

fn apply_update_with_head(
    repo: &mut Repository,
    head_oid: &str,
    message: &str,
    changes: Vec<ReviewedContentChange>,
    previous_config: Option<RepoConfig>,
    config: RepoConfig,
) {
    let mut update = reviewed_update(head_oid, message, changes, previous_config, config);
    if let Some(previous) = repo.git_head.as_ref() {
        let sequence = previous.push_sequence + 1;
        update.git_head = scope_domain::repository::git::GitHead::new(
            head_oid.to_string(),
            sequence,
            update.git_head.change_version,
        );
        update.git_pack_span.first_sequence = sequence;
        update.git_pack_span.last_sequence = sequence;
        update.git_pack_span.base_oid = Some(previous.head_oid.clone());
    }
    apply_reviewed_update_to_repo(repo, update).unwrap();
}

fn reviewed_update(
    head_oid: &str,
    message: &str,
    changes: Vec<ReviewedContentChange>,
    previous_config: Option<RepoConfig>,
    config: RepoConfig,
) -> ReviewedUpdateInput {
    ReviewedUpdateInput {
        occurred_at_unix: None,
        branch: "main".to_string(),
        author_id: "owner".to_string(),
        message: message.to_string(),
        git_head: scope_domain::repository::git::GitHead::new(head_oid.to_string(), 1, 1),
        git_pack_span: scope_domain::repository::git::GitPackSpan {
            first_sequence: 1,
            last_sequence: 1,
            geometric_tier: 0,
            base_oid: None,
            head_oid: head_oid.to_string(),
            segment: scope_domain::repository::git::GitSegmentRef {
                segment_id: "segment-v2".to_string(),
                sha256: "sha256:segment v2".to_string(),
                plaintext_bytes: 10,
                encoding_version: 2,
            },
        },
        changes,
        previous_config,
        open_requests_by_view: Default::default(),
        config,
    }
}

#[test]
fn reviewed_push_rejects_a_pack_span_that_does_not_advance_the_current_frontier() {
    let mut repo = published_repo_with_public_file("initial", "/README.md", "hello");
    let config = repo.repo_config.clone();
    apply_update_with_head(
        &mut repo,
        "1111111111111111111111111111111111111111",
        "first push",
        vec![reviewed_change("/README.md", Some("second"))],
        Some(config.clone()),
        config.clone(),
    );

    let stale = reviewed_update(
        "2222222222222222222222222222222222222222",
        "stale push",
        vec![reviewed_change("/README.md", Some("third"))],
        Some(config.clone()),
        config,
    );
    let error = apply_reviewed_update_to_repo(&mut repo, stale).unwrap_err();

    assert!(matches!(
        error,
        ReviewedUpdateError::Conflict("Git push does not advance the current pack frontier")
    ));
}

#[test]
fn content_push_command_returns_normalized_effects_without_previous_config() {
    let repo = published_repo_with_public_file("initial", "/README.md", "hello");
    let config = config(
        ViewId::public(),
        Some(("/secret.txt", ViewId::private())),
        None,
    );
    let accepted = accept_content_push(
        ContentPushState {
            change_version: repo.record.change_version,
            content_version: repo.record.content_version,
            policy: repo.policy.clone(),
            repo_config: config.clone(),
            live_files: repo.live_files.clone(),
            git_head: repo.git_head.clone(),
        },
        reviewed_update(
            "3333333333333333333333333333333333333333",
            "add secret",
            vec![reviewed_change("/secret.txt", Some("secret"))],
            None,
            config,
        ),
    )
    .unwrap();

    assert_eq!(accepted.change_version, 2);
    assert_eq!(accepted.git_head.change_version, 2);
    assert_eq!(
        accepted.logical_commit.origin,
        LogicalCommitOrigin::CanonicalPush {
            source_head_oid: "3333333333333333333333333333333333333333".to_string(),
        }
    );
    assert_eq!(accepted.logical_commit.changes.len(), 1);
    assert_eq!(
        accepted
            .policy
            .label(&path("/secret.txt"), &Views::builtin()),
        ViewId::private()
    );
    assert_eq!(repo.record.change_version, 1);
    assert_eq!(repo.graph.commits.len(), 1);
}

#[test]
fn request_merge_requires_an_ordered_native_range_inside_its_view() {
    let repo = published_repo_with_public_file("initial", "/README.md", "hello");
    let config = repo.repo_config.clone();
    let state = ContentPushState {
        change_version: repo.record.change_version,
        content_version: repo.record.content_version,
        policy: repo.policy.clone(),
        repo_config: config.clone(),
        live_files: repo.live_files.clone(),
        git_head: repo.git_head.clone(),
    };
    let update = reviewed_update(
        "3333333333333333333333333333333333333333",
        "merge public request",
        vec![reviewed_change("/README.md", Some("contributor edit"))],
        Some(config.clone()),
        config,
    );
    let commits = vec![
        NativeRequestCommit {
            oid: "1111111111111111111111111111111111111111".to_string(),
            parent_oids: vec!["9999999999999999999999999999999999999999".to_string()],
            tree_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            changed_paths: vec![ScopePath::parse("/README.md").unwrap()],
        },
        NativeRequestCommit {
            oid: "2222222222222222222222222222222222222222".to_string(),
            parent_oids: vec![
                "1111111111111111111111111111111111111111".to_string(),
                "0000000000000000000000000000000000000000".to_string(),
            ],
            tree_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            changed_paths: vec![ScopePath::parse("/README.md").unwrap()],
        },
    ];
    let origin = RequestMergeOrigin::View {
        request_id: "request-1".to_string(),
        view: ViewId::public(),
        base_oid: "0000000000000000000000000000000000000000".to_string(),
        parent_oids: vec![
            "0000000000000000000000000000000000000000".to_string(),
            "9999999999999999999999999999999999999999".to_string(),
        ],
        request_head_oid: "2222222222222222222222222222222222222222".to_string(),
        commits: commits.clone(),
    };

    let accepted = accept_request_merge(state.clone(), update.clone(), origin).unwrap();
    assert_eq!(
        accepted.logical_commit.origin,
        LogicalCommitOrigin::RequestMerge {
            request_id: "request-1".to_string(),
            view: ViewId::public(),
            base_oid: "0000000000000000000000000000000000000000".to_string(),
            parent_oids: vec![
                "0000000000000000000000000000000000000000".to_string(),
                "9999999999999999999999999999999999999999".to_string(),
            ],
            request_head_oid: "2222222222222222222222222222222222222222".to_string(),
            commits: commits.clone(),
            preserve_commits: true,
        }
    );

    let mut broken_commits = commits.clone();
    broken_commits[0].parent_oids = vec!["2222222222222222222222222222222222222222".to_string()];
    assert!(
        accept_request_merge(
            state.clone(),
            update.clone(),
            RequestMergeOrigin::View {
                request_id: "request-1".to_string(),
                view: ViewId::public(),
                base_oid: "0000000000000000000000000000000000000000".to_string(),
                parent_oids: vec![
                    "0000000000000000000000000000000000000000".to_string(),
                    "9999999999999999999999999999999999999999".to_string(),
                ],
                request_head_oid: "2222222222222222222222222222222222222222".to_string(),
                commits: broken_commits,
            },
        )
        .is_err()
    );

    let mut external_parent_commits = commits;
    external_parent_commits[0].parent_oids =
        vec!["8888888888888888888888888888888888888888".to_string()];
    assert!(matches!(
        accept_request_merge(
            state,
            update,
            RequestMergeOrigin::View {
                request_id: "request-1".to_string(),
                view: ViewId::public(),
                base_oid: "0000000000000000000000000000000000000000".to_string(),
                parent_oids: vec![
                    "0000000000000000000000000000000000000000".to_string(),
                    "9999999999999999999999999999999999999999".to_string(),
                ],
                request_head_oid: "2222222222222222222222222222222222222222".to_string(),
                commits: external_parent_commits,
            },
        ),
        Err(ReviewedUpdateError::Domain(error))
            if error.message == "request merge contains a parent outside Public history"
    ));
}

#[test]
fn request_merge_rejects_paths_outside_its_view() {
    let cases = [
        (
            "reviewed private change",
            "/secret.txt",
            "/secret.txt",
            "mixed request",
            true,
        ),
        (
            "private intermediate native path",
            "/secret/**",
            "/secret/transient.txt",
            "request with transient private path",
            false,
        ),
    ];

    for (case, private_rule, changed_path, message, include_reviewed_change) in cases {
        let repo = published_repo_with_public_file("initial", "/README.md", "hello");
        let config = config(
            ViewId::public(),
            Some((private_rule, ViewId::private())),
            None,
        );
        let changes = include_reviewed_change
            .then(|| reviewed_change(changed_path, Some("secret")))
            .into_iter()
            .collect();
        let result = accept_request_merge(
            ContentPushState {
                change_version: repo.record.change_version,
                content_version: repo.record.content_version,
                policy: repo.policy.clone(),
                repo_config: config.clone(),
                live_files: repo.live_files.clone(),
                git_head: repo.git_head.clone(),
            },
            reviewed_update(
                "3333333333333333333333333333333333333333",
                message,
                changes,
                Some(config.clone()),
                config,
            ),
            RequestMergeOrigin::View {
                request_id: "request-1".to_string(),
                view: ViewId::public(),
                base_oid: "0000000000000000000000000000000000000000".to_string(),
                parent_oids: vec!["0000000000000000000000000000000000000000".to_string()],
                request_head_oid: "1111111111111111111111111111111111111111".to_string(),
                commits: vec![NativeRequestCommit {
                    oid: "1111111111111111111111111111111111111111".to_string(),
                    parent_oids: vec!["0000000000000000000000000000000000000000".to_string()],
                    tree_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                    changed_paths: vec![ScopePath::parse(changed_path).unwrap()],
                }],
            },
        );

        assert!(result.is_err(), "{case}");
    }
}

#[test]
fn content_only_updates_keep_policy_and_commit_identity_in_sync() {
    let mut repo = published_repo_with_public_file("initial", "/README.md", "hello");
    let config = config(
        ViewId::public(),
        Some(("/secret.txt", ViewId::private())),
        None,
    );
    repo.repo_config = config.clone();

    apply_update_with_head(
        &mut repo,
        "3333333333333333333333333333333333333333",
        "add secret",
        vec![reviewed_change("/secret.txt", Some("secret"))],
        Some(config.clone()),
        config.clone(),
    );
    apply_update_with_head(
        &mut repo,
        "4444444444444444444444444444444444444444",
        "update readme",
        vec![reviewed_change("/README.md", Some("updated"))],
        Some(config.clone()),
        config,
    );

    assert_eq!(
        repo.policy.label(&path("/secret.txt"), &Views::builtin()),
        ViewId::private()
    );
    assert_eq!(
        repo.graph.commits[repo.graph.commits.len() - 2].id,
        "rv_push_3333333333333333333333333333333333333333"
    );
    assert_eq!(
        repo.graph.commits.last().unwrap().id,
        "rv_push_4444444444444444444444444444444444444444"
    );
}

#[test]
fn content_only_update_preserves_existing_visibility_override() {
    let mut repo = published_repo_with_public_file("initial", "/README.md", "hello");
    repo.policy
        .add_rule(LabelRule::private(path("/README.md")))
        .unwrap();
    let config = repo.repo_config.clone();

    apply_update(
        &mut repo,
        "update readme",
        vec![reviewed_change("/README.md", Some("updated"))],
        Some(config.clone()),
        config,
    );

    assert_eq!(
        repo.policy.label(&path("/README.md"), &Views::builtin()),
        ViewId::private()
    );
    assert_eq!(
        repo.graph.commits.last().unwrap().changes[0].label,
        ViewId::private()
    );
    assert!(repo.visibility_change_sets.is_empty());
}

#[test]
fn config_only_update_changes_policy_without_content_commit() {
    let mut repo = published_test_repo(ViewId::private());
    repo.graph.commits.push(commit(
        "rv1",
        "initial",
        added("/README.md", ViewId::private(), "hello"),
    ));
    repo.live_files.insert(path("/README.md"), blob("hello"));

    let changed = apply_reviewed_config_to_repo(
        &mut repo,
        ReviewedConfigUpdateInput {
            occurred_at_unix: 1_788_700_000,
            author_id: "owner".to_string(),
            open_requests_by_view: Default::default(),
            config: config(
                ViewId::private(),
                Some(("/README.md", ViewId::public())),
                None,
            ),
        },
    )
    .unwrap();

    assert!(changed);
    assert_eq!(repo.graph.commits.len(), 1);
    assert_eq!(
        repo.policy.label(&path("/README.md"), &Views::builtin()),
        ViewId::public()
    );
    assert_eq!(repo.visibility_change_sets.len(), 1);
    assert_eq!(repo.visibility_change_sets[0].source_update_id, None);
    assert_eq!(
        repo.visibility_change_sets[0].occurred_at_unix,
        Some(1_788_700_000)
    );
    assert_eq!(
        repo.repo_config.label_for_path(&path("/README.md")),
        ViewId::public()
    );
}

#[test]
fn public_projection_contains_only_visible_paths_from_mixed_commit() {
    let mut mixed = commit(
        "rv1",
        "mixed",
        added("/README.md", ViewId::public(), "hello"),
    );
    mixed
        .changes
        .push(added("/internal/model.rs", ViewId::private(), "secret"));
    let graph = graph(vec![mixed]);

    let projection = project_graph(&graph, &[], &Views::builtin(), &ViewId::public());

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.visible_paths(), vec!["/README.md"]);
    assert_eq!(projection.commits[0].message, "Projected public update");
    assert!(projection.commits[0].author.is_none());
}

#[test]
fn public_request_origin_expands_to_exact_native_commits_only_in_public_view() {
    let mut request_merge = commit(
        "rv_merge_canonical",
        "merge public request",
        added("/README.md", ViewId::public(), "contributor version"),
    );
    request_merge.origin = LogicalCommitOrigin::RequestMerge {
        request_id: "request-1".to_string(),
        view: ViewId::public(),
        base_oid: "base".to_string(),
        parent_oids: vec!["base".to_string()],
        request_head_oid: "r2".to_string(),
        preserve_commits: true,
        commits: vec![
            NativeRequestCommit {
                oid: "r1".to_string(),
                parent_oids: vec!["base".to_string()],
                tree_oid: "tree-1".to_string(),
                changed_paths: vec![ScopePath::parse("/README.md").unwrap()],
            },
            NativeRequestCommit {
                oid: "r2".to_string(),
                parent_oids: vec!["r1".to_string()],
                tree_oid: "tree-2".to_string(),
                changed_paths: vec![ScopePath::parse("/README.md").unwrap()],
            },
        ],
    };
    let graph = graph(vec![request_merge]);
    let public = project_graph(&graph, &[], &Views::builtin(), &ViewId::public());
    assert_eq!(
        public
            .commits
            .iter()
            .map(|commit| commit.projected_id.as_str())
            .collect::<Vec<_>>(),
        ["r1", "r2"]
    );
    assert!(public.commits[0].changes.is_empty());
    assert_eq!(public.commits[1].changes.len(), 1);
    assert!(matches!(
        &public.commits[0].materialization,
        ProjectionMaterialization::PreserveGitCommit { oid, .. } if oid == "r1"
    ));
    assert!(matches!(
        &public.commits[1].materialization,
        ProjectionMaterialization::PreserveGitCommit { oid, .. } if oid == "r2"
    ));

    let private = project_graph(&graph, &[], &Views::builtin(), &ViewId::private());
    assert_eq!(private.commits.len(), 1);
    assert_eq!(
        private.commits[0].materialization,
        ProjectionMaterialization::Generate
    );

    let mut definitions = Vec::<scope_domain::views::ViewDefinition>::from(Views::builtin());
    definitions.push(scope_domain::views::ViewDefinition {
        id: ViewId::parse("agent").unwrap(),
        name: "Agent".into(),
        includes: scope_domain::views::ViewIncludes::Some([ViewId::public()].into()),
        readers: scope_domain::views::ViewReaders::Assigned,
    });
    let with_agent = Views::new(definitions).unwrap();
    let agent = project_graph(&graph, &[], &with_agent, &ViewId::parse("agent").unwrap());
    assert_eq!(agent.commits.len(), 1);
    assert_eq!(
        agent.commits[0].materialization,
        ProjectionMaterialization::Generate
    );
    assert_eq!(agent.visible_paths(), vec!["/README.md"]);
}

#[test]
fn public_projection_keeps_public_history_when_a_later_edit_is_private() {
    let graph = graph(vec![
        commit(
            "rv1",
            "public readme",
            added("/README.md", ViewId::public(), "public readme"),
        ),
        commit(
            "rv2",
            "private readme",
            change(
                "/README.md",
                ViewId::private(),
                Some(blob("public readme")),
                Some(blob("private readme")),
            ),
        ),
    ]);

    let projection = project_graph(&graph, &[], &Views::builtin(), &ViewId::public());

    assert_eq!(projection.commits.len(), 1);
    assert_eq!(projection.commits[0].logical_commit_id, "rv1");
    assert_eq!(projection.visible_paths(), vec!["/README.md"]);
}

#[test]
fn public_projection_never_contains_tracked_workflow_definitions() {
    let graph = graph(vec![commit(
        "rv1",
        "add workflow",
        added("/.scope/runs/test.yml", ViewId::public(), "name: Test"),
    )]);

    let projection = project_graph(&graph, &[], &Views::builtin(), &ViewId::public());

    assert!(projection.visible_paths().is_empty());
}

#[test]
fn redacting_intermediate_native_path_invalidates_descendant_commit_preservation() {
    let mut repo = published_repo_with_public_file("initial", "/README.md", "hello");
    let mut request_merge = commit(
        "rv_request_merge",
        "merge public request",
        added("/kept.txt", ViewId::public(), "kept"),
    );
    request_merge.origin = LogicalCommitOrigin::RequestMerge {
        request_id: "request-1".to_string(),
        view: ViewId::public(),
        base_oid: "base".to_string(),
        parent_oids: vec!["base".to_string()],
        request_head_oid: "request-head".to_string(),
        commits: vec![NativeRequestCommit {
            oid: "request-head".to_string(),
            parent_oids: vec!["base".to_string()],
            tree_oid: "request-tree".to_string(),
            changed_paths: vec![
                ScopePath::parse("/transient-leak.txt").unwrap(),
                ScopePath::parse("/kept.txt").unwrap(),
            ],
        }],
        preserve_commits: true,
    };
    repo.graph.commits.push(request_merge);
    repo.live_files.insert(path("/kept.txt"), blob("kept"));
    let mut later_request_merge = commit(
        "rv_later_request_merge",
        "merge later public request",
        added("/later.txt", ViewId::public(), "later"),
    );
    later_request_merge.origin = LogicalCommitOrigin::RequestMerge {
        request_id: "request-2".to_string(),
        view: ViewId::public(),
        base_oid: "request-head".to_string(),
        parent_oids: vec!["request-head".to_string()],
        request_head_oid: "later-request-head".to_string(),
        commits: vec![NativeRequestCommit {
            oid: "later-request-head".to_string(),
            parent_oids: vec!["request-head".to_string()],
            tree_oid: "later-request-tree".to_string(),
            changed_paths: vec![ScopePath::parse("/later.txt").unwrap()],
        }],
        preserve_commits: true,
    };
    repo.graph.commits.push(later_request_merge);
    repo.live_files.insert(path("/later.txt"), blob("later"));

    apply_update(
        &mut repo,
        "redact transient path",
        vec![reviewed_change("/.scope/runs/test.yml", Some("name: Test"))],
        None,
        config(ViewId::public(), None, Some("/transient-leak.txt")),
    );

    let projection = project_repo(&repo, ViewId::public());
    assert!(
        projection
            .commits
            .iter()
            .all(|commit| commit.materialization == ProjectionMaterialization::Generate)
    );
    assert_eq!(
        projection.visible_paths(),
        vec!["/README.md", "/kept.txt", "/later.txt"]
    );
    assert!(matches!(
        &repo.graph.commits[1].origin,
        LogicalCommitOrigin::RequestMerge {
            preserve_commits: false,
            ..
        }
    ));
    assert!(matches!(
        &repo.graph.commits[2].origin,
        LogicalCommitOrigin::RequestMerge {
            preserve_commits: false,
            ..
        }
    ));
}

#[test]
fn unchanged_history_rewrite_is_not_reapplied_on_later_push() {
    let config = config(ViewId::public(), None, Some("/leaked.txt"));
    let mut repo = published_repo_with_public_file(
        "existing public history",
        "/leaked.txt",
        "existing public content",
    );

    apply_update(
        &mut repo,
        "later config-only push",
        vec![reviewed_change("/.scope/runs/test.yml", Some("name: Test"))],
        Some(config.clone()),
        config,
    );

    let public_projection = project_repo(&repo, ViewId::public());

    assert_eq!(public_projection.commits.len(), 1);
    assert_eq!(public_projection.commits[0].logical_commit_id, "rv1");
    assert_eq!(public_projection.visible_paths(), vec!["/leaked.txt"]);
}

#[test]
fn public_projection_handles_reveal_and_private_gap_timelines() {
    let cases = [
        (
            vec![
                ("rv1", ViewId::private(), "draft"),
                ("rv2", ViewId::public(), "release"),
            ],
            vec![],
            vec!["rv2"],
        ),
        (
            vec![("rv1", ViewId::private(), "draft")],
            vec![("vis_1", Some("rv1"), None, ViewId::public(), "draft")],
            vec!["vis_1"],
        ),
        (
            vec![
                ("rv1", ViewId::public(), "v1"),
                ("rv2", ViewId::private(), "v2"),
                ("rv3", ViewId::public(), "v3"),
            ],
            vec![
                ("vis_1", Some("rv1"), Some("rv2"), ViewId::private(), "v2"),
                ("vis_2", None, Some("rv3"), ViewId::public(), "v3"),
            ],
            vec!["rv1", "rv2", "rv3"],
        ),
        (
            vec![("rv1", ViewId::public(), "readme")],
            vec![
                ("vis_1", Some("rv1"), None, ViewId::private(), "readme"),
                ("vis_2", Some("rv1"), None, ViewId::public(), "readme"),
            ],
            vec!["rv1", "vis_1", "vis_2"],
        ),
    ];
    for (versions, events, expected_ids) in cases {
        let projection = project_timeline("/file", &versions, &events);
        assert_eq!(
            projection
                .commits
                .iter()
                .map(|commit| commit.logical_commit_id.as_str())
                .collect::<Vec<_>>(),
            expected_ids
        );
        assert_eq!(projection.visible_paths(), vec!["/file"]);
    }
}
