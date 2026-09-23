use super::*;
use crate::git::{
    cache::RepositoryGitCache,
    command::{git_command_output, git_process_output, run_git},
    import::git_snapshot_from_ref,
};
use scope_domain::{
    content::SourceBlob,
    requests::{RequestActorRole, RequestAudience},
};

fn request(name: &str, head: &str, git_snapshot: Option<SourceBlob>) -> Request {
    Request {
        id: format!("request-{name}"),
        repo_id: "repo".into(),
        name: name.into(),
        author_user_id: "author".into(),
        author_role: RequestActorRole::Public,
        audience: RequestAudience::Public,
        base_main_oid: head.into(),
        head_oid: head.into(),
        git_snapshot,
        title: name.into(),
        description_markdown: String::new(),
        activity_version: 0,
        submitted_at_unix: Some(1),
        closed_at_unix: None,
        closed_by_user_id: None,
        merged_at_unix: None,
        merged_by_user_id: None,
        merged_head_oid: None,
        merged_main_oid: None,
        created_at_unix: 1,
        updated_at_unix: 1,
    }
}

fn ref_head(path: &FsPath, refname: &str) -> String {
    let head = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(path)
            .arg("rev-parse")
            .arg(refname),
        None,
    )
    .unwrap();
    String::from_utf8(head).unwrap().trim().to_string()
}

fn commit_in(path: &FsPath, message: &str, parent: &str) -> String {
    let tree = git_command_output(
        Command::new("git").arg("--git-dir").arg(path).arg("mktree"),
        Some(b""),
    )
    .unwrap();
    let oid = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(path)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .arg("commit-tree")
            .arg(String::from_utf8(tree).unwrap().trim())
            .arg("-p")
            .arg(parent)
            .arg("-m")
            .arg(message),
        None,
    )
    .unwrap();
    String::from_utf8(oid).unwrap().trim().to_string()
}

/// Advances the request branch `refs/heads/<request>` in `source` by one commit and stores
/// its snapshot bundle, the way a request push does.
async fn advance_request_snapshot(
    state: &AppState,
    source: &FsPath,
    request: &str,
    message: &str,
) -> (String, SourceBlob) {
    let request_ref = format!("refs/heads/{request}");
    let parent = git_process_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(source)
            .arg("rev-parse")
            .arg("--verify")
            .arg(&request_ref),
        None,
        ProcessLimits::new(Duration::from_secs(5)),
    )
    .unwrap();
    let parent = if parent.status.success() {
        String::from_utf8(parent.stdout).unwrap().trim().to_string()
    } else {
        ref_head(source, "refs/heads/main")
    };
    let head = commit_in(source, message, &parent);
    run_git(
        Some(source),
        &["update-ref", &request_ref, &head],
        "advance request branch",
    )
    .unwrap();
    let (snapshot, bytes) = git_snapshot_from_ref(source, &request_ref, None).unwrap();
    state
        .object_store
        .put(&scope_storage::object_key(&snapshot), bytes)
        .await
        .unwrap();
    (head, snapshot)
}

fn base_repo(cache: &std::sync::Arc<RepositoryGitCache>, name: &str) -> GitRepoHandle {
    let path = cache.root().join(format!("{name}.git"));
    run_git(
        None,
        &["init", "--bare", path.to_str().unwrap()],
        "init base",
    )
    .unwrap();
    run_git(
        Some(&path),
        &["symbolic-ref", "HEAD", "refs/heads/main"],
        "set head",
    )
    .unwrap();
    let tree = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&path)
            .arg("mktree"),
        Some(b""),
    )
    .unwrap();
    let oid = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&path)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .arg("commit-tree")
            .arg(String::from_utf8(tree).unwrap().trim())
            .arg("-m")
            .arg(name),
        None,
    )
    .unwrap();
    run_git(
        Some(&path),
        &[
            "update-ref",
            "refs/heads/main",
            String::from_utf8(oid).unwrap().trim(),
        ],
        "set base",
    )
    .unwrap();
    cache.lease_derived(path).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_base_head_change_rebuilds_read_view_and_attaches_newly_available_ref() {
    let state = AppState::test_state();
    let source_dir = tempfile::tempdir().unwrap();
    let cache = RepositoryGitCache::new(source_dir.path().to_path_buf(), usize::MAX).unwrap();
    let primary = base_repo(&cache, "primary");
    let public_first = base_repo(&cache, "public-first");
    let public_second = base_repo(&cache, "public-second");
    let second_path = public_second.as_ref().to_path_buf();
    let head = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&second_path)
            .arg("rev-parse")
            .arg("refs/heads/main"),
        None,
    )
    .unwrap();
    let head = String::from_utf8(head).unwrap().trim().to_string();
    let requests = vec![request("topic", &head, None)];
    let incarnation = RepositoryIncarnation::new("repo", "incarnation").unwrap();
    let primary_path = primary.as_ref().to_path_buf();
    let first = git_read_view_repo(&state, &incarnation, primary, Some(public_first), &requests)
        .await
        .unwrap();
    let missing = git_process_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(first.as_ref())
            .arg("rev-parse")
            .arg("--verify")
            .arg("refs/heads/topic"),
        None,
        ProcessLimits::new(Duration::from_secs(5)),
    )
    .unwrap();
    assert!(!missing.status.success());
    let second = git_read_view_repo(
        &state,
        &incarnation,
        cache.lease_derived(primary_path).unwrap(),
        Some(public_second),
        &requests,
    )
    .await
    .unwrap();
    assert_ne!(first.as_ref(), second.as_ref());
    let attached = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(second.as_ref())
            .arg("rev-parse")
            .arg("refs/heads/topic"),
        None,
    )
    .unwrap();
    assert_eq!(String::from_utf8(attached).unwrap().trim(), head);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unchanged_request_refs_are_copied_from_earlier_read_views() {
    let state = AppState::test_state();
    let source_dir = tempfile::tempdir().unwrap();
    let cache = RepositoryGitCache::new(source_dir.path().to_path_buf(), usize::MAX).unwrap();
    let primary = base_repo(&cache, "primary");
    let primary_path = primary.as_ref().to_path_buf();
    let main_head = ref_head(&primary_path, "refs/heads/main");
    let topic_source = base_repo(&cache, "topic-source");
    let (topic_head, snapshot) =
        advance_request_snapshot(&state, topic_source.as_ref(), "alpha", "topic").await;
    let incarnation = RepositoryIncarnation::new("repo", "incarnation").unwrap();
    let alpha = request("alpha", &topic_head, Some(snapshot.clone()));

    let first = git_read_view_repo(
        &state,
        &incarnation,
        primary,
        None,
        std::slice::from_ref(&alpha),
    )
    .await
    .unwrap();
    assert_eq!(ref_head(first.as_ref(), "refs/heads/alpha"), topic_head);

    // Once the snapshot is gone from the object store, only the first read view can supply alpha.
    state
        .object_store
        .delete(&scope_storage::object_key(&snapshot))
        .await
        .unwrap();
    let requests = vec![alpha, request("beta", &main_head, None)];
    let second = git_read_view_repo(
        &state,
        &incarnation,
        cache.lease_derived(primary_path).unwrap(),
        None,
        &requests,
    )
    .await
    .unwrap();

    assert_ne!(first.as_ref(), second.as_ref());
    assert_eq!(ref_head(second.as_ref(), "refs/heads/alpha"), topic_head);
    assert_eq!(ref_head(second.as_ref(), "refs/heads/beta"), main_head);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn earlier_read_views_only_seed_the_exact_request_head() {
    let state = AppState::test_state();
    let source_dir = tempfile::tempdir().unwrap();
    let cache = RepositoryGitCache::new(source_dir.path().to_path_buf(), usize::MAX).unwrap();
    let primary = base_repo(&cache, "primary");
    let primary_path = primary.as_ref().to_path_buf();
    let topic_source = base_repo(&cache, "topic-source");
    let (first_head, first_snapshot) =
        advance_request_snapshot(&state, topic_source.as_ref(), "alpha", "topic one").await;
    let incarnation = RepositoryIncarnation::new("repo", "incarnation").unwrap();

    let first = git_read_view_repo(
        &state,
        &incarnation,
        primary,
        None,
        &[request("alpha", &first_head, Some(first_snapshot))],
    )
    .await
    .unwrap();
    assert_eq!(ref_head(first.as_ref(), "refs/heads/alpha"), first_head);

    let (second_head, second_snapshot) =
        advance_request_snapshot(&state, topic_source.as_ref(), "alpha", "topic two").await;
    let second = git_read_view_repo(
        &state,
        &incarnation,
        cache.lease_derived(primary_path).unwrap(),
        None,
        &[request("alpha", &second_head, Some(second_snapshot))],
    )
    .await
    .unwrap();

    assert_ne!(first.as_ref(), second.as_ref());
    assert_eq!(ref_head(second.as_ref(), "refs/heads/alpha"), second_head);
}
