use super::*;
use scope_domain::repository::git::GitPackSpan;
use scope_git_process::ProcessError;
use scope_storage::{EncryptionKey, GitSegmentStoreConfig, MemoryBackend};
use std::io::Cursor;

fn oid(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().trim().to_string()
}

fn commit_worktree(repo: &Path, message: &str) -> String {
    let repo = repo.to_string_lossy();
    run_git(
        None,
        &["-C", &repo, "add", "-A"],
        None,
        Duration::from_secs(2),
        1024,
    )
    .unwrap();
    run_git(
        None,
        &[
            "-C",
            &repo,
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@test.invalid",
            "commit",
            "-m",
            message,
        ],
        None,
        Duration::from_secs(2),
        1024,
    )
    .unwrap();
    oid(run_git(
        None,
        &["-C", &repo, "rev-parse", "HEAD"],
        None,
        Duration::from_secs(2),
        1024,
    )
    .unwrap())
}

fn make_pack(repo: &Path, head: &str, base: Option<&str>) -> Vec<u8> {
    let revisions = match base {
        Some(base) => format!("{head}\n^{base}\n"),
        None => format!("{head}\n"),
    };
    run_git(
        Some(repo),
        &["pack-objects", "--revs", "--stdout"],
        Some(revisions.into_bytes()),
        Duration::from_secs(2),
        1024 * 1024,
    )
    .unwrap()
}

fn make_pack_with_extra_objects(
    repo: &Path,
    head: &str,
    base: &str,
    extra_object_ids: &[&str],
) -> Vec<u8> {
    let revisions = format!("{head}\n^{base}\n");
    let listed = run_git(
        Some(repo),
        &["rev-list", "--objects", "--stdin"],
        Some(revisions.into_bytes()),
        Duration::from_secs(2),
        1024 * 1024,
    )
    .unwrap();
    let mut object_ids = String::from_utf8(listed)
        .unwrap()
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(|object_id| parse_object_id(object_id).unwrap())
        .collect::<BTreeSet<_>>();
    object_ids.extend(
        extra_object_ids
            .iter()
            .map(|object_id| parse_object_id(object_id).unwrap()),
    );
    run_git(
        Some(repo),
        &["pack-objects", "--stdout"],
        Some(object_id_input(&object_ids)),
        Duration::from_secs(2),
        1024 * 1024,
    )
    .unwrap()
}

async fn span(
    store: &GitSegmentStore,
    repository_id: &str,
    sequences: (u64, u64),
    tier: u32,
    boundary: (Option<String>, String),
    pack: Vec<u8>,
) -> GitPackSpan {
    let staged = store
        .ingest_blocking_reader(repository_id, Cursor::new(pack), u64::MAX)
        .await
        .unwrap();
    GitPackSpan {
        first_sequence: sequences.0,
        last_sequence: sequences.1,
        geometric_tier: tier,
        base_oid: boundary.0,
        head_oid: boundary.1,
        segment: staged.segment,
    }
}

fn segment_store(local_root: &Path) -> Arc<GitSegmentStore> {
    let mut config = GitSegmentStoreConfig::new(local_root);
    config.chunk_bytes = 1024;
    config.multipart_part_bytes = 1024;
    Arc::new(
        GitSegmentStore::new(
            Arc::new(MemoryBackend::default()),
            EncryptionKey::new("test", [7_u8; 32]).unwrap(),
            config,
        )
        .unwrap(),
    )
}

#[test]
fn worker_git_output_obeys_exact_byte_limit() {
    let exact = run_git(
        None,
        &["hash-object", "--stdin"],
        Some(b"content".to_vec()),
        Duration::from_secs(1),
        41,
    )
    .unwrap();
    assert_eq!(exact.len(), 41);

    let error = run_git(
        None,
        &["hash-object", "--stdin"],
        Some(b"content".to_vec()),
        Duration::from_secs(1),
        40,
    )
    .unwrap_err();
    assert!(
        error
            .downcast_ref::<ProcessError>()
            .is_some_and(ProcessError::is_stdout_limit)
    );
}

#[tokio::test]
async fn compaction_preserves_the_selected_packs_exact_object_set() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    run_git(
        None,
        &["init", source.to_string_lossy().as_ref()],
        None,
        Duration::from_secs(2),
        1024,
    )
    .unwrap();
    fs::write(source.join("changing.txt"), "version 1\n").unwrap();
    fs::write(source.join("deleted.txt"), "remove me\n").unwrap();
    fs::write(source.join("restored.txt"), "restore me\n").unwrap();
    let head_1 = commit_worktree(&source, "one");

    fs::write(source.join("changing.txt"), "version 2\n").unwrap();
    fs::remove_file(source.join("restored.txt")).unwrap();
    let head_2 = commit_worktree(&source, "two");

    fs::write(source.join("changing.txt"), "version 3\n").unwrap();
    fs::remove_file(source.join("deleted.txt")).unwrap();
    fs::write(source.join("restored.txt"), "restore me\n").unwrap();
    fs::write(source.join("transient.txt"), "only in three\n").unwrap();
    let head_3 = commit_worktree(&source, "three");

    fs::write(source.join("changing.txt"), "version 4\n").unwrap();
    fs::remove_file(source.join("transient.txt")).unwrap();
    let head_4 = commit_worktree(&source, "four");
    let git_dir = source.join(".git");
    let restored_blob = oid(run_git(
        Some(&git_dir),
        &["rev-parse", &format!("{head_1}:restored.txt")],
        None,
        Duration::from_secs(2),
        1024,
    )
    .unwrap());
    let predecessor_pack = make_pack(&git_dir, &head_2, None);
    let first_selected_pack =
        make_pack_with_extra_objects(&git_dir, &head_3, &head_2, &[&restored_blob]);
    let second_selected_pack = make_pack(&git_dir, &head_4, Some(&head_3));

    let expected = TemporaryGitRepo::new(temp.path()).unwrap();
    run_git(
        None,
        &["init", "--bare", expected.path.to_string_lossy().as_ref()],
        None,
        Duration::from_secs(2),
        1024,
    )
    .unwrap();
    for pack in [&first_selected_pack, &second_selected_pack] {
        run_git(
            Some(&expected.path),
            &["index-pack", "--stdin"],
            Some(pack.clone()),
            Duration::from_secs(2),
            1024,
        )
        .unwrap();
    }
    let expected_object_ids =
        enumerate_object_ids(&expected.path, Duration::from_secs(2), 1024 * 1024).unwrap();

    let repository_id = "owner/repo";
    let incarnation = RepositoryIncarnation::new(repository_id, "repoi_compaction").unwrap();
    let store = segment_store(&temp.path().join("segments"));
    let predecessor = span(
        store.as_ref(),
        repository_id,
        (1, 2),
        1,
        (None, head_2.clone()),
        predecessor_pack,
    )
    .await;
    let selected = vec![
        span(
            store.as_ref(),
            repository_id,
            (3, 3),
            0,
            (Some(head_2.clone()), head_3.clone()),
            first_selected_pack,
        )
        .await,
        span(
            store.as_ref(),
            repository_id,
            (4, 4),
            0,
            (Some(head_3.clone()), head_4.clone()),
            second_selected_pack,
        )
        .await,
    ];
    for selected_span in &selected {
        store
            .cleanup_local(repository_id, &selected_span.segment.segment_id)
            .await
            .unwrap();
    }
    let mut layout = vec![predecessor];
    layout.extend(selected);
    let plan = GitCompactionPlan::select(&layout, u64::MAX)
        .unwrap()
        .unwrap();

    let reservation = store.reserve(repository_id).unwrap();
    let compacted = build_compacted_pack(
        Arc::clone(&store),
        &incarnation,
        &plan,
        reservation,
        GitStorageLimits::new(1024 * 1024).unwrap(),
        Duration::from_secs(2),
        temp.path().to_path_buf(),
    )
    .await
    .unwrap();
    assert_eq!(compacted.metrics.local_restore_count, 0);
    assert_eq!(compacted.metrics.remote_restore_count, 2);
    let compacted_bytes = fs::read(compacted.staged.local_pack_path()).unwrap();

    let result = TemporaryGitRepo::new(temp.path()).unwrap();
    run_git(
        None,
        &["init", "--bare", result.path.to_string_lossy().as_ref()],
        None,
        Duration::from_secs(2),
        1024,
    )
    .unwrap();
    run_git(
        Some(&result.path),
        &["index-pack", "--stdin"],
        Some(compacted_bytes),
        Duration::from_secs(2),
        1024,
    )
    .unwrap();
    let actual_object_ids =
        enumerate_object_ids(&result.path, Duration::from_secs(2), 1024 * 1024).unwrap();
    assert_eq!(actual_object_ids, expected_object_ids);
    assert!(actual_object_ids.contains(&parse_object_id(&restored_blob).unwrap()));
    assert!(!actual_object_ids.contains(&parse_object_id(&head_2).unwrap()));
    assert!(actual_object_ids.contains(&parse_object_id(&head_3).unwrap()));
    assert!(actual_object_ids.contains(&parse_object_id(&head_4).unwrap()));
}

#[test]
fn temporary_git_repositories_stay_under_the_worker_data_directory() {
    let temp = tempfile::tempdir().unwrap();
    let repo = TemporaryGitRepo::new(temp.path()).unwrap();

    assert!(repo.path.starts_with(temp.path().join("git-compaction")));
}
