use super::*;
use crate::db::{CatalogFixture, MetadataStore, TestDatabaseTarget};
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    history::history_view_from_projection,
    policy::ScopePath,
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin, SourceGraph, project_graph},
    repository::{RepoLifecycleState, Repository},
    views::{ViewId, Views},
    visibility_changes::VisibilityChangeSet,
};
use std::time::Duration;

fn history_view(
    graph: &SourceGraph,
    sets: &[VisibilityChangeSet],
    view_key: ViewId,
) -> HistoryView {
    let views = Views::builtin();
    history_view_from_projection(
        project_graph(graph, sets, &views, &view_key),
        graph,
        sets,
        &views,
        &view_key,
    )
}

fn fixture(commits: usize) -> (MetadataStore, Repository) {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let owner = UserAccount {
        id: "history_owner".into(),
        handle: "owner".into(),
        email: "history@example.com".into(),
        email_verified: true,
    };
    let mut repo = Repository::new(&owner, "history", ViewId::public(), "repoi_history").unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    for index in 0..commits {
        let oid = format!("{:040x}", index + 1);
        repo.graph.commits.push(LogicalCommit {
            occurred_at_unix: None,
            id: format!("logical_{index}"),
            origin: LogicalCommitOrigin::CanonicalPush {
                source_head_oid: oid.clone(),
            },
            author_id: owner.id.clone(),
            message: format!("Change {index}"),
            changes: vec![FileChange {
                path: ScopePath::parse(format!("/file-{}.txt", index % 32)).unwrap(),
                old_content: None,
                new_content: Some(SourceBlob {
                    content_ref: ContentRef::git_bundle_sha256(format!("bundle-{index}")),
                    sha256: format!("hash-{index}"),
                    git_oid: oid,
                    git_file_mode: "100644".into(),
                    size_bytes: 100,
                }),
                label: if index % 2 == 0 {
                    ViewId::public()
                } else {
                    ViewId::private()
                },
            }],
        });
    }
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog
        .repositories
        .insert(repo.record.id.clone(), repo.clone());
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    (store, repo)
}

#[tokio::test]
async fn history_pages_match_domain_projection_and_do_not_read_history_when_warm() {
    let (store, repo) = fixture(1000);
    store
        .db
        .execute_unprepared("DELETE FROM scope_projection_read_models")
        .await
        .unwrap();
    let hydrated = store
        .repositories()
        .repository("owner", "history")
        .await
        .unwrap()
        .unwrap();
    let expected_private = history_view(
        &hydrated.graph,
        &hydrated.visibility_change_sets,
        ViewId::private(),
    );
    let first = store
        .repositories()
        .repository_history_page(RepositoryHistoryQuery {
            incarnation: &repo.incarnation(),
            change_version: repo.record.change_version,
            view: &ViewId::private(),
            feed: HistoryFeed::All,
            before: None,
            entry_source_id: None,
            limit: 50,
        })
        .await
        .unwrap();
    assert!(first.next_boundary.is_some());
    assert_eq!(first.view.entries, expected_private.entries[..50]);
    assert_eq!(
        first.view.generation,
        HistoryFeed::All.generation(&expected_private.generation, &repo.record.id, "private")
    );

    let held = store.db.begin().await.unwrap();
    held.execute_unprepared("LOCK TABLE scope_logical_commits, scope_file_changes, scope_live_files IN ACCESS EXCLUSIVE MODE").await.unwrap();
    let next = tokio::time::timeout(
        Duration::from_secs(2),
        store
            .repositories()
            .repository_history_page(RepositoryHistoryQuery {
                incarnation: &repo.incarnation(),
                change_version: repo.record.change_version,
                view: &ViewId::private(),
                feed: HistoryFeed::All,
                before: first.next_boundary.as_ref(),
                entry_source_id: None,
                limit: 50,
            }),
    )
    .await
    .expect("warm pages must not hydrate source history")
    .unwrap();
    assert_eq!(next.view.entries, expected_private.entries[50..100]);
    let public_access = tokio::time::timeout(
        Duration::from_secs(2),
        store
            .repositories()
            .repository_read_access("owner", "history", None),
    )
    .await
    .expect("public access must reuse current projection facts")
    .unwrap()
    .unwrap();
    let expected_public = history_view(&repo.graph, &repo.visibility_change_sets, ViewId::public());
    let public = store
        .repositories()
        .repository_history_page(RepositoryHistoryQuery {
            incarnation: &public_access.incarnation(),
            change_version: public_access.record.change_version,
            view: &public_access.access.view,
            feed: HistoryFeed::All,
            before: None,
            entry_source_id: None,
            limit: 50,
        })
        .await
        .unwrap();
    assert_eq!(public.view.entries, expected_public.entries[..50]);
    assert_eq!(
        public.view.generation,
        HistoryFeed::All.generation(&expected_public.generation, &repo.record.id, "public")
    );
    let detail = store
        .repositories()
        .repository_history_page(RepositoryHistoryQuery {
            incarnation: &repo.incarnation(),
            change_version: repo.record.change_version,
            view: &ViewId::public(),
            feed: HistoryFeed::All,
            before: None,
            entry_source_id: Some(&public.view.entries[10].source_id),
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(detail.view.entries, vec![public.view.entries[10].clone()]);
    held.rollback().await.unwrap();
}

#[tokio::test]
async fn actions_group_repeated_projection_sources_and_page_by_exact_position() {
    use scope_domain::visibility_changes::{VisibilityChange, VisibilityChangeSet};

    let (store, mut repo) = fixture(5);
    repo.visibility_change_sets.push(
        VisibilityChangeSet::new(
            "vchg_split".into(),
            Some("logical_0".into()),
            Some("logical_4".into()),
            "history_owner".into(),
            vec![VisibilityChange {
                path: repo.graph.commits[0].changes[0].path.clone(),
                old_label: ViewId::public(),
                new_label: ViewId::private(),
                current_content: repo.graph.commits[0].changes[0].new_content.clone(),
            }],
        )
        .unwrap(),
    );
    repo.bump_content_version();
    let expected = history_view(&repo.graph, &repo.visibility_change_sets, ViewId::public());
    assert_eq!(
        expected
            .entries
            .iter()
            .map(|entry| entry.source_id.as_str())
            .collect::<Vec<_>>(),
        ["logical_4", "logical_2", "logical_0"],
    );
    assert_eq!(expected.entries[0].visibility_changes.len(), 1);

    store
        .repositories()
        .replace_repository_for_tests(repo.clone())
        .await
        .unwrap();
    let rebuilt = store
        .jobs()
        .run_ready_outbox_jobs("history-regression", 10, &|| Ok(1_700_000_000))
        .await
        .unwrap();
    assert!(rebuilt.completed > 0);
    assert_eq!(rebuilt.failed, 0);
    assert!(
        live_projection_read_model(
            store.db.as_ref(),
            &repo.record.id,
            repo.record.content_version,
            &ViewId::public(),
        )
        .await
        .unwrap()
        .is_some()
    );
    store
        .db
        .execute_unprepared("DELETE FROM scope_projection_read_models")
        .await
        .unwrap();
    let mut before = None;
    let mut collected = Vec::new();
    let mut first_boundary = None;
    for expected_entry in &expected.entries {
        let page = store
            .repositories()
            .repository_history_page(RepositoryHistoryQuery {
                incarnation: &repo.incarnation(),
                change_version: repo.record.change_version,
                view: &ViewId::public(),
                feed: HistoryFeed::All,
                before: before.as_ref(),
                entry_source_id: None,
                limit: 1,
            })
            .await
            .unwrap();
        assert_eq!(page.view.entries, vec![expected_entry.clone()]);
        assert_eq!(
            page.view.generation,
            HistoryFeed::All.generation(&expected.generation, &repo.record.id, "public")
        );
        if collected.is_empty() {
            first_boundary = page.next_boundary.clone();
        }
        collected.extend(page.view.entries);
        before = page.next_boundary;
    }
    assert!(before.is_none());
    assert_eq!(collected, expected.entries);

    let detail = store
        .repositories()
        .repository_history_page(RepositoryHistoryQuery {
            incarnation: &repo.incarnation(),
            change_version: repo.record.change_version,
            view: &ViewId::public(),
            feed: HistoryFeed::All,
            before: None,
            entry_source_id: Some("logical_4"),
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(detail.view.entries, vec![expected.entries[0].clone()]);

    let mut next_commit = repo.graph.commits.last().unwrap().clone();
    next_commit.id = "logical_5".into();
    next_commit.message = "Another update".into();
    next_commit.changes[0].path = ScopePath::parse("/another-file.txt").unwrap();
    repo.graph.commits.push(next_commit);
    repo.bump_content_version();
    store
        .repositories()
        .replace_repository_for_tests(repo.clone())
        .await
        .unwrap();
    let stale = store
        .repositories()
        .repository_history_page(RepositoryHistoryQuery {
            incarnation: &repo.incarnation(),
            change_version: repo.record.change_version,
            view: &ViewId::public(),
            feed: HistoryFeed::All,
            before: first_boundary.as_ref(),
            entry_source_id: None,
            limit: 1,
        })
        .await
        .err()
        .expect("a position must not be reused in another generation");
    assert!(
        stale
            .message
            .contains("history changed; restart pagination")
    );
}

#[tokio::test]
async fn history_reads_reject_changed_frontiers_and_deleted_boundaries() {
    let (store, repo) = fixture(4);
    store
        .repositories()
        .ensure_live_projection_read_models(&repo.incarnation())
        .await
        .unwrap();
    store
        .db
        .execute_unprepared("UPDATE scope_projection_read_models SET history_version='stale'")
        .await
        .unwrap();
    store
        .repositories()
        .ensure_live_projection_read_models(&repo.incarnation())
        .await
        .unwrap();
    assert!(
        live_projection_read_model(
            store.db.as_ref(),
            &repo.record.id,
            repo.record.content_version,
            &ViewId::private()
        )
        .await
        .unwrap()
        .is_some()
    );
    let missing = store
        .repositories()
        .repository_history_page(RepositoryHistoryQuery {
            incarnation: &repo.incarnation(),
            change_version: repo.record.change_version,
            view: &ViewId::private(),
            feed: HistoryFeed::All,
            before: Some(&RepositoryHistoryBoundary {
                generation: HistoryFeed::All.generation(
                    &history_view(&repo.graph, &repo.visibility_change_sets, ViewId::private())
                        .generation,
                    &repo.record.id,
                    "private",
                ),
                position: 999,
            }),
            entry_source_id: None,
            limit: 50,
        })
        .await;
    assert!(
        missing
            .err()
            .unwrap()
            .message
            .contains("boundary is no longer available")
    );
    let previous_context = store
        .repositories()
        .repository_access("owner", "history", Some("history_owner"))
        .await
        .unwrap()
        .unwrap();
    store.db.execute_unprepared("UPDATE scope_repositories SET change_version=change_version+1 WHERE id='owner/history'").await.unwrap();
    assert!(
        store
            .repositories()
            .repository_main_oid(&previous_context)
            .await
            .is_err()
    );
    assert!(
        store
            .repositories()
            .repository_history_page(RepositoryHistoryQuery {
                incarnation: &repo.incarnation(),
                change_version: repo.record.change_version,
                view: &ViewId::private(),
                feed: HistoryFeed::All,
                before: None,
                entry_source_id: None,
                limit: 50
            })
            .await
            .is_err()
    );
    let current = store
        .repositories()
        .repository_access("owner", "history", Some("history_owner"))
        .await
        .unwrap()
        .unwrap();
    store.db.execute_unprepared("UPDATE scope_repositories SET incarnation_id='repoi_recreated' WHERE id='owner/history'").await.unwrap();
    assert!(
        store
            .repositories()
            .repository_main_oid(&current)
            .await
            .is_err()
    );
    assert!(
        store
            .repositories()
            .repository_policy(&current)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn narrow_access_preserves_membership_lifecycle_and_public_root_capabilities() {
    use scope_domain::{
        policy::{LabelRule, Policy},
        repository::collaboration::{RepositoryMember, RepositoryMemberPermissions},
    };
    let (store, mut repo) = fixture(4);
    repo.bump_content_version();
    repo.policy = Policy::new(ViewId::private());
    repo.policy
        .add_rule(LabelRule::public(ScopePath::parse("/file-0.txt").unwrap()))
        .unwrap();
    repo.collaboration.members.push(RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: "member".into(),
        permissions: RepositoryMemberPermissions {
            can_push: true,
            can_change_file_visibility: false,
            view: ViewId::private(),
        },
        created_at_unix: 1,
        updated_at_unix: 1,
    });
    store
        .repositories()
        .replace_repository_for_tests(repo.clone())
        .await
        .unwrap();
    for user in [
        None,
        Some("history_owner"),
        Some("member"),
        Some("outsider"),
    ] {
        let narrow = store
            .repositories()
            .repository_read_access("owner", "history", user)
            .await
            .unwrap()
            .unwrap();
        let expected = user
            .map(|user| repo.access_for_user_id(user))
            .unwrap_or_else(scope_domain::repository::access::RepositoryAccess::public);
        assert_eq!(narrow.access, expected);
        if user.is_none() {
            let public = store
                .repositories()
                .repository_history_page(RepositoryHistoryQuery {
                    incarnation: &narrow.incarnation(),
                    change_version: narrow.record.change_version,
                    view: &narrow.access.view,
                    feed: HistoryFeed::All,
                    before: None,
                    entry_source_id: None,
                    limit: 50,
                })
                .await
                .unwrap();
            assert_eq!(
                public.view.entries,
                history_view(&repo.graph, &repo.visibility_change_sets, ViewId::public()).entries
            );
            assert_ne!(
                public.view.entries,
                history_view(&repo.graph, &repo.visibility_change_sets, ViewId::private()).entries
            );
        }
    }
    assert!(
        scope_domain::projection_views::has_visible_projected_non_control_files(
            &repo,
            repo.repo_config.views(),
            &ViewId::public()
        )
    );
    store.db.execute_unprepared("UPDATE scope_repositories SET publication_state='AwaitingFirstPush', change_version=change_version+1 WHERE id='owner/history'").await.unwrap();
    assert!(
        store
            .repositories()
            .repository_read_access("owner", "history", None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .repositories()
            .repository_read_access("owner", "history", Some("member"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .repositories()
            .repository_access("owner", "history", Some("member"))
            .await
            .unwrap()
            .unwrap()
            .ensure_member()
            .is_ok()
    );
    assert!(
        store
            .repositories()
            .repository_read_access("owner", "history", Some("history_owner"))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn feed_filters_before_limit_and_binds_boundaries() {
    use scope_domain::visibility_changes::{VisibilityChange, VisibilityChangeSet};
    let (store, mut repo) = fixture(55);
    for index in 0..60 {
        let (old_visibility, new_visibility) = if index % 2 == 0 {
            (ViewId::public(), ViewId::private())
        } else {
            (ViewId::private(), ViewId::public())
        };
        repo.visibility_change_sets.push(
            VisibilityChangeSet::new(
                format!("visibility_{index}"),
                Some("logical_54".into()),
                None,
                "history_owner".into(),
                vec![VisibilityChange {
                    path: repo.graph.commits[0].changes[0].path.clone(),
                    old_label: old_visibility,
                    new_label: new_visibility,
                    current_content: repo.graph.commits[32].changes[0].new_content.clone(),
                }],
            )
            .unwrap(),
        );
    }
    repo.bump_content_version();
    store
        .repositories()
        .replace_repository_for_tests(repo.clone())
        .await
        .unwrap();
    let incarnation = repo.incarnation();
    let private_view = ViewId::private();
    let query = |feed, before| RepositoryHistoryQuery {
        incarnation: &incarnation,
        change_version: repo.record.change_version,
        view: &private_view,
        feed,
        before,
        entry_source_id: None,
        limit: 50,
    };
    let first = store
        .repositories()
        .repository_history_page(query(HistoryFeed::Updates, None))
        .await
        .unwrap();
    assert_eq!(first.view.entries.len(), 50);
    assert_eq!(first.view.entries[0].source_id, "logical_54");
    assert!(
        first
            .view
            .entries
            .iter()
            .all(|entry| entry.kind != scope_domain::history::HistoryEntryKind::VisibilityChange)
    );
    let next = store
        .repositories()
        .repository_history_page(query(HistoryFeed::Updates, first.next_boundary.as_ref()))
        .await
        .unwrap();
    assert_eq!(next.view.entries.len(), 5);
    assert!(next.next_boundary.is_none());
    let all = store
        .repositories()
        .repository_history_page(query(HistoryFeed::All, None))
        .await
        .unwrap();
    assert_eq!(all.view.entries.len(), 50);
    assert!(
        all.view
            .entries
            .iter()
            .all(|entry| entry.kind == scope_domain::history::HistoryEntryKind::VisibilityChange)
    );
    assert_ne!(all.view.generation, first.view.generation);
    let mismatch = store
        .repositories()
        .repository_history_page(query(HistoryFeed::All, first.next_boundary.as_ref()))
        .await
        .err()
        .unwrap();
    assert!(mismatch.message.contains("history changed"));
    let detail = store
        .repositories()
        .repository_history_page(RepositoryHistoryQuery {
            entry_source_id: Some("visibility_59"),
            ..query(HistoryFeed::All, None)
        })
        .await
        .unwrap();
    assert_eq!(detail.view.entries[0].source_id, "visibility_59");
    assert!(store.db.execute_unprepared("INSERT INTO scope_repository_history_entries (repo_id, audience, position, source_id, payload_hash) SELECT repo_id, audience, position + 10000, source_id, payload_hash FROM scope_repository_history_entries LIMIT 1").await.is_err());
}

#[tokio::test]
async fn a_push_folds_only_its_own_commits_onto_the_read_models() {
    let (store, mut repo) = fixture(300);
    store
        .jobs()
        .run_ready_outbox_jobs("history-fold", 10, &|| Ok(1_700_000_000))
        .await
        .unwrap();
    let pushed = LogicalCommit {
        occurred_at_unix: Some(1_700_000_001),
        id: "logical_300".into(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: format!("{:040x}", 301),
        },
        author_id: "history_owner".into(),
        message: "Change 300".into(),
        changes: vec![FileChange {
            path: ScopePath::parse("/file-0.txt").unwrap(),
            old_content: repo.graph.commits[288].changes[0].new_content.clone(),
            new_content: Some(SourceBlob {
                content_ref: ContentRef::git_bundle_sha256("bundle-300"),
                sha256: "hash-300".into(),
                git_oid: format!("{:040x}", 301),
                git_file_mode: "100644".into(),
                size_bytes: 100,
            }),
            label: ViewId::public(),
        }],
    };
    repo.graph.commits.push(pushed.clone());
    repo.live_files.insert(
        pushed.changes[0].path.clone(),
        pushed.changes[0].new_content.clone().unwrap(),
    );
    repo.bump_content_version();
    store
        .repositories()
        .replace_repository_for_tests(repo.clone())
        .await
        .unwrap();
    store
        .db
        .execute_unprepared("DELETE FROM scope_file_changes WHERE commit_id <> 'logical_300'")
        .await
        .unwrap();
    let rebuilt = store
        .jobs()
        .run_ready_outbox_jobs("history-fold", 10, &|| Ok(1_700_000_002))
        .await
        .unwrap();
    assert_eq!(rebuilt.failed, 0, "{rebuilt:?}");

    for view_key in [ViewId::private(), ViewId::public()] {
        let expected = history_view(&repo.graph, &repo.visibility_change_sets, view_key.clone());
        let page = store
            .repositories()
            .repository_history_page(RepositoryHistoryQuery {
                incarnation: &repo.incarnation(),
                change_version: repo.record.change_version,
                view: &view_key,
                feed: HistoryFeed::All,
                before: None,
                entry_source_id: None,
                limit: 50,
            })
            .await
            .unwrap();
        assert_eq!(page.view.entries, expected.entries[..50]);
        assert_eq!(
            page.view.generation,
            HistoryFeed::All.generation(&expected.generation, &repo.record.id, view_key.as_str())
        );
        assert_eq!(
            page.head_oid,
            scope_git::projection_head_oid(&project_graph(
                &repo.graph,
                &repo.visibility_change_sets,
                repo.repo_config.views(),
                &view_key
            ))
            .unwrap()
        );
        let files = store
            .repositories()
            .repo_live_files(
                "owner",
                "history",
                (view_key == ViewId::private()).then_some("history_owner"),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            files.len(),
            if view_key == ViewId::private() {
                32
            } else {
                16
            }
        );
        assert!(files.iter().all(|file| repo.repo_config.views().shows(
            &view_key,
            &file.path,
            &file.label
        )));
    }
}
