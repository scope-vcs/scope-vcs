use super::main_pool::main_pool_packs;
use super::*;
use scope_domain::views::ViewId;

async fn repo_with_push_member(
    state: &AppState,
    member_id: &str,
    permissions: RepositoryMemberPermissions,
) {
    let mut repo = repo_with_readme(state);
    repo.collaboration
        .members
        .push(test_repository_member(TEST_REPO_ID, member_id, permissions));
    replace_test_repo(state, repo).await;
}

fn commit_readme(repo: &FsPath, content: &str, message: &str) {
    fs::write(repo.join("README.md"), content).unwrap();
    run_git(Some(repo), &["add", "-A"], "stage readme change").unwrap();
    commit_all(repo, message);
}

#[tokio::test]
async fn published_receive_pack_push_applies_from_seeded_git_repo() {
    let state = test_state_with_repo();
    let mut repo = repo_with_readme(&state);
    repo.graph.commits[0].changes.push(FileChange {
        label: ViewId::public(),
        path: ScopePath::parse("/unchanged.md").unwrap(),
        old_content: None,
        new_content: Some(source_blob(&state, "already here")),
    });
    replace_test_repo(&state, repo).await;
    let staging_repo = published_staging_repo(&state).await;
    let clone = clone_test_repo(&staging_repo, "published-push-clone", false);
    fs::write(clone.join("README.md"), "staged readme").unwrap();
    fs::write(clone.join("notes.md"), "new notes").unwrap();
    run_git(Some(&clone), &["add", "-A"], "stage clone changes").unwrap();
    commit_all(&clone, "update from git");
    run_git(
        Some(&clone),
        &["push", "origin", DEFAULT_GIT_BRANCH],
        "push applied update",
    )
    .unwrap();

    let update = reviewed_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &staging_repo,
        &test_owner_id(),
        repo_config(ViewId::public()),
        ReviewedUpdateMode::ReadyPush,
    )
    .await
    .unwrap();

    assert_eq!(update.branch, format!("refs/heads/{DEFAULT_GIT_BRANCH}"));
    assert_eq!(update.message, "update from git");
    let expected_commit_time: i64 = git_stdout_text(
        &clone,
        &["log", "-1", "--format=%ct"],
        "read source commit time",
    )
    .unwrap()
    .trim()
    .parse()
    .unwrap();
    assert_eq!(update.occurred_at_unix, Some(expected_commit_time));
    persist_test_update(&state, update).await.unwrap();
    let persisted = state
        .metadata
        .repositories()
        .repository(TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        persisted.graph.commits.last().unwrap().occurred_at_unix,
        Some(expected_commit_time)
    );
    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("staged readme")
    );
    assert_eq!(
        live_file_content(&state, "/notes.md").await.as_deref(),
        Some("new notes")
    );
    let _ = fs::remove_dir_all(&staging_repo);
}

#[tokio::test]
async fn consecutive_content_only_pushes_advance_the_live_projection() {
    let state = test_state_with_repo();
    let mut repo = repo_with_readme(&state);
    let base_head_oid = "0000000000000000000000000000000000000001".to_string();
    let base_segment = ready_test_git_segment(&state, "base Git segment").await;
    let base_pack = scope_domain::repository::git::GitPoolPack {
        id: base_segment.segment_id.clone(),
        owner: scope_domain::repository::git::GitPoolPackOwner::Main {
            first_sequence: 1,
            last_sequence: 1,
        },
        head_oid: base_head_oid.clone(),
        prerequisites: Vec::new(),
        storage: scope_domain::repository::git::GitPoolPackStorage::Segment(base_segment),
    };
    repo.git_head = Some(scope_domain::repository::git::GitHead::new(
        base_head_oid.clone(),
        1,
        repo.record.change_version,
    ));
    let mut previous_frontier = repo.git_head.as_ref().unwrap().frontier();
    replace_test_repo(&state, repo).await;
    state
        .metadata
        .repositories()
        .register_main_pool_pack_for_tests(TEST_REPO_ID, &base_pack)
        .await
        .unwrap();

    let initial_rebuild = drain_outbox(&state, "content-push-test").await;
    assert_eq!(initial_rebuild.failed, 0);
    assert_eq!(initial_rebuild.completed, 1);

    let mut previous_head_oid = base_head_oid;
    for (index, expected_content) in ["second version", "third version"].into_iter().enumerate() {
        let sequence = u64::try_from(index + 2).unwrap();
        let head_oid = format!("{sequence:040x}");
        let mut update = receive_pack_update(&state, vec![("/README.md", Some(expected_content))]);
        update.previous_config = Some(update.config.clone());
        update.base_git_frontier = Some(Some(previous_frontier));
        update.head_oid = head_oid.clone();
        update.git_head =
            scope_domain::repository::git::GitHead::new(head_oid.clone(), sequence, sequence);
        update.main_pack = scope_domain::repository::git::GitPoolPack {
            id: format!("main-pack-{sequence}"),
            owner: scope_domain::repository::git::GitPoolPackOwner::Main {
                first_sequence: sequence,
                last_sequence: sequence,
            },
            head_oid: head_oid.clone(),
            prerequisites: vec![previous_head_oid],
            storage: scope_domain::repository::git::GitPoolPackStorage::Segment(
                test_git_segment_ref(&format!("Git segment {sequence}")),
            ),
        };
        update.workflow_catalog = scope_domain::runs::catalog::RepositoryWorkflowCatalog::captured(
            TEST_REPO_ID,
            &head_oid,
            sequence,
            Vec::new(),
        )
        .unwrap();

        let next_frontier = update.git_head.frontier();
        let persisted = persist_test_update(&state, update).await.unwrap();
        assert_eq!(persisted.change_version, sequence);
        let stored = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap();
        assert_eq!(stored.record.change_version, sequence);
        assert_eq!(stored.git_head.unwrap().change_version, sequence);

        let deferred = drain_outbox(&state, "content-push-test").await;
        assert_eq!(deferred.claimed, 0);
        let projected = state
            .metadata
            .repositories()
            .repo_live_file_with_landing_content(
                TEST_REPO_OWNER,
                TEST_REPO_NAME,
                None,
                None,
                &ScopePath::parse("/README.md").unwrap(),
            )
            .await
            .unwrap()
            .unwrap()
            .projected;
        let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap();
        assert_eq!(
            blob_content(&state, &projected.blob, &repo).await,
            expected_content
        );

        previous_head_oid = head_oid;
        previous_frontier = next_frontier;
    }

    let jobs = state
        .metadata
        .jobs()
        .outbox_job_counts_for_tests()
        .await
        .unwrap();
    assert_eq!(jobs.succeeded, 1);
    assert_eq!(jobs.total, 1);

    let now_unix = crate::persistence::unix_now().unwrap();
    let compaction = state
        .metadata
        .jobs()
        .claim_git_compaction("content-push-test", u64::MAX, now_unix, 30)
        .await
        .unwrap()
        .expect("accepted pushes schedule durable compaction work");
    assert_eq!(compaction.target_sequence, 3);
    let selected = compaction
        .candidate
        .as_ref()
        .expect("an equal-tier pair is eligible after two pushes")
        .plan
        .selected_packs();
    assert_eq!(
        (
            selected[0].owner.main_sequences().unwrap().0,
            selected[1].owner.main_sequences().unwrap().1
        ),
        (1, 2)
    );
    state
        .metadata
        .jobs()
        .complete_git_compaction_claim(&compaction, now_unix)
        .await
        .unwrap();
}

#[tokio::test]
async fn published_receive_pack_rejects_non_fast_forward_push() {
    let state = test_state_with_repo();
    let staging_repo = published_staging_repo(&state).await;
    let clone = clone_test_repo(&staging_repo, "published-force-push-clone", false);

    commit_readme(&clone, "fast forward readme", "fast-forward update");
    run_git(
        Some(&clone),
        &["push", "origin", DEFAULT_GIT_BRANCH],
        "push fast-forward update",
    )
    .unwrap();
    let accepted_head = git_stdout_text(
        &staging_repo,
        &["rev-parse", DEFAULT_GIT_BRANCH],
        "read accepted head",
    )
    .unwrap();

    run_git(
        Some(&clone),
        &["reset", "--hard", "HEAD~1"],
        "rewind clone before force push",
    )
    .unwrap();
    commit_readme(&clone, "rewritten readme", "rewritten update");
    let output = run_git_output(
        Some(&clone),
        &["push", "--force", "origin", DEFAULT_GIT_BRANCH],
        "force push rewritten update",
    )
    .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Scope rejects non-fast-forward pushes")
    );
    let current_head = git_stdout_text(
        &staging_repo,
        &["rev-parse", DEFAULT_GIT_BRANCH],
        "read current head",
    )
    .unwrap();
    assert_eq!(current_head, accepted_head);
    let _ = fs::remove_dir_all(&staging_repo);
}

#[tokio::test]
async fn push_only_member_can_apply_content_without_visibility_changes() {
    let state = test_state_with_repo();
    let member_id = "user_push_only";
    repo_with_push_member(&state, member_id, member_permissions(true, false)).await;

    let persisted = persist_and_promote_test_update(
        &state,
        receive_pack_update(&state, vec![("/README.md", Some("hello\nextra line"))]),
        member_id,
    )
    .await
    .unwrap();

    assert!(!persisted.head_oid.is_empty());
    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("hello\nextra line")
    );
}

#[tokio::test]
async fn published_push_rechecks_member_permission_before_persisting() {
    let state = test_state_with_repo();
    let member_id = "user_removed_during_push";
    repo_with_push_member(&state, member_id, member_permissions(true, false)).await;
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, move |repo| {
            repo.collaboration
                .members
                .retain(|member| member.user_id != member_id);
        })
        .await
        .unwrap();

    let error = persist_and_promote_test_update(
        &state,
        receive_pack_update(&state, vec![("/README.md", Some("should not persist"))]),
        member_id,
    )
    .await
    .unwrap_err();

    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        live_file_content(&state, "/README.md").await.as_deref(),
        Some("hello")
    );
}

#[tokio::test]
async fn published_receive_pack_staging_restores_accepted_git_head_from_bucket_snapshot() {
    let state = test_state_with_repo();
    let source = temp_git_repo("snapshot-first-push");
    fs::write(source.join("README.md"), "hello from git").unwrap();
    run_git(Some(&source), &["add", "README.md"], "add readme").unwrap();
    commit_all(&source, "initial from git");
    let bare = clone_test_repo(&source, "snapshot-first-push-bare", true);
    let expected_head =
        git_stdout_text(&bare, &["rev-parse", DEFAULT_GIT_BRANCH], "first push head").unwrap();
    apply_first_push_from_staging_repo(&state, &bare, repo_config(ViewId::public())).await;

    let restored =
        ensure_ready_receive_pack_staging_repo(&state, &owner_push_context(&state).await)
            .await
            .unwrap()
            .path;
    let actual_head = git_stdout_text(
        &restored,
        &["rev-parse", DEFAULT_GIT_BRANCH],
        "restored head",
    )
    .unwrap();

    assert_eq!(actual_head, expected_head);
    let _ = fs::remove_dir_all(&restored);
}

#[tokio::test]
async fn applying_push_retains_previous_main_pool_pack() {
    let state = test_state_with_repo();
    let old_head_oid = "0000000000000000000000000000000000000001".to_string();
    let old_segment = ready_test_git_segment(&state, "old live Git pack").await;
    let mut update = receive_pack_update(&state, vec![("/README.md", Some("updated"))]);
    update.git_head = scope_domain::repository::git::GitHead::new(
        update.git_head.head_oid.clone(),
        2,
        update.git_head.change_version,
    );
    update.main_pack.owner = scope_domain::repository::git::GitPoolPackOwner::Main {
        first_sequence: 2,
        last_sequence: 2,
    };
    update.main_pack.prerequisites = vec![old_head_oid.clone()];
    let mut repo = repo_with_readme(&state);
    repo.git_head = Some(scope_domain::repository::git::GitHead::new(
        old_head_oid.clone(),
        1,
        1,
    ));
    replace_test_repo(&state, repo).await;
    state
        .metadata
        .repositories()
        .register_main_pool_pack_for_tests(
            TEST_REPO_ID,
            &scope_domain::repository::git::GitPoolPack {
                id: old_segment.segment_id.clone(),
                owner: scope_domain::repository::git::GitPoolPackOwner::Main {
                    first_sequence: 1,
                    last_sequence: 1,
                },
                head_oid: old_head_oid.clone(),
                prerequisites: Vec::new(),
                storage: scope_domain::repository::git::GitPoolPackStorage::Segment(
                    old_segment.clone(),
                ),
            },
        )
        .await
        .unwrap();

    let persisted = persist_and_promote_test_update(&state, update, &test_owner_id())
        .await
        .unwrap();

    assert!(!persisted.head_oid.is_empty());
    let listed = main_pool_packs(&state).await;
    assert_eq!(
        listed
            .iter()
            .map(|pack| (pack.first_sequence, pack.prerequisites.clone()))
            .collect::<Vec<_>>(),
        [(1, Vec::new()), (2, vec![old_head_oid])]
    );
    assert_eq!(listed[0].id, old_segment.segment_id);
    let storage = state
        .metadata
        .repositories()
        .git_pool_pack_storage(
            TEST_REPO_ID,
            &listed
                .iter()
                .map(|pack| pack.id.clone())
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    assert_eq!(storage.len(), 2);
    for (_, storage) in storage {
        let segment = storage.segment().unwrap();
        state
            .git_segment_store
            .restore_to(TEST_REPO_ID, segment, tokio::io::sink())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn pending_object_cleanup_uses_transactional_reference_rows() {
    let state = test_state_with_repo();
    let live_blob = source_blob(&state, "referenced pending content");
    {
        let mut repo = repo_with_readme(&state);
        repo.graph.commits[0].changes[0].new_content = Some(live_blob.clone());
        replace_test_repo(&state, repo).await;
        state
            .metadata
            .cleanup()
            .queue_pending_source_blob_deletions(vec![live_blob.clone()], unix_now())
            .await
            .unwrap();
    }
    state
        .metadata
        .cleanup()
        .expire_source_blob_cleanup_grace_for_tests()
        .await
        .unwrap();

    let report = drain_pending_source_blob_deletions(&state).await.unwrap();

    assert_eq!(
        report,
        scope_content_lifecycle::SourceBlobCleanupReport {
            skipped_referenced: 1,
            ..Default::default()
        }
    );

    assert!(
        state
            .test_object_backend
            .object(&scope_storage::object_key(&live_blob))
            .is_some()
    );
    assert!(
        state
            .metadata
            .cleanup()
            .pending_source_blob_cleanups_for_tests()
            .await
            .unwrap()
            .is_empty()
    );
}
