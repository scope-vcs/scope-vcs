use super::*;

#[derive(Clone, Copy)]
pub(super) struct SeedGitCommit<'a> {
    pub(super) files: &'a [(&'a str, &'a str)],
    pub(super) message: &'a str,
}

pub(super) fn git_pack_state(
    git_segment_store: &scope_git_storage::GitSegmentStore,
    repository_id: &str,
    label: &str,
    commits: &[SeedGitCommit<'_>],
) -> Result<(GitHead, GitPackSpan, GitSegmentUpload), ApiError> {
    with_seed_git_repo(label, |repo_path| {
        apply_seed_commits(repo_path, commits)?;
        store_seed_git_pack(git_segment_store, repository_id, repo_path)
    })
}

pub(super) fn update_demo_git_snapshot(
    object_store: &dyn ObjectStore,
    git_segment_store: &scope_git_storage::GitSegmentStore,
    repository_id: &str,
    initial: SeedGitCommit<'_>,
    accepted: SeedGitCommit<'_>,
) -> Result<(GitHead, GitPackSpan, SeedRequestGallery, GitSegmentUpload), ApiError> {
    with_seed_git_repo("update-demo-live", |repo_path| {
        apply_seed_commits(repo_path, &[initial])?;
        let initial_oid = seed_git_head(repo_path)?;
        apply_seed_commits(repo_path, &[accepted])?;
        let main_oid = seed_git_head(repo_path)?;
        let accepted_ref = canonical_request_ref("document-release-flow");
        seed_git(
            Some(repo_path),
            &["update-ref", &accepted_ref, &main_oid],
            "creating seeded request ref",
        )?;

        let ready_oid = seed_request_branch(
            repo_path,
            "bounded-retry-timing",
            SeedGitCommit {
                files: &[("src/retry.ts", UPDATE_DEMO_RETRY_HELPER)],
                message: "Add bounded retry timing",
            },
            &main_oid,
        )?;
        let ready_snapshot = store_seed_bundle(
            object_store,
            repo_path,
            "req_demo_ready_0",
            &[&canonical_request_ref("bounded-retry-timing")],
            &ready_oid,
        )?;
        let ready_revisions = request_revisions::seed_bounded_retry_revisions(
            object_store,
            repo_path,
            &ready_oid,
            &main_oid,
        )?;
        let held_oid = seed_request_branch(
            repo_path,
            "remote-troubleshooting",
            SeedGitCommit {
                files: &[("docs/troubleshooting.md", UPDATE_DEMO_TROUBLESHOOTING)],
                message: "Add remote troubleshooting",
            },
            &main_oid,
        )?;
        let rejected_oid = seed_request_branch(
            repo_path,
            "verbose-cli-output",
            SeedGitCommit {
                files: &[("experiments/cli-output.txt", UPDATE_DEMO_CLI_EXPERIMENT)],
                message: "Try verbose CLI output",
            },
            &main_oid,
        )?;
        let working_oid = seed_request_branch(
            repo_path,
            "request-queue-copy",
            SeedGitCommit {
                files: &[("docs/request-queue.md", UPDATE_DEMO_QUEUE_DRAFT)],
                message: "Draft request queue copy",
            },
            &main_oid,
        )?;
        let neutral_oid = seed_request_branch(
            repo_path,
            "cache-observability-note",
            SeedGitCommit {
                files: &[("docs/cache-note.md", UPDATE_DEMO_CACHE_NOTE)],
                message: "Document cache tradeoff",
            },
            &main_oid,
        )?;
        let (main_head, main_pack_span, segment_upload) =
            store_seed_git_pack(git_segment_store, repository_id, repo_path)?;
        let working_snapshot = store_seed_bundle(
            object_store,
            repo_path,
            "req_demo_working",
            &[&canonical_request_ref("request-queue-copy")],
            &working_oid,
        )?;
        let held_snapshot = store_seed_bundle(
            object_store,
            repo_path,
            "req_demo_held",
            &[&canonical_request_ref("remote-troubleshooting")],
            &held_oid,
        )?;
        let accepted_snapshot = store_seed_bundle(
            object_store,
            repo_path,
            "req_demo_accepted",
            &[&accepted_ref],
            &main_oid,
        )?;
        let rejected_snapshot = store_seed_bundle(
            object_store,
            repo_path,
            "req_demo_rejected",
            &[&canonical_request_ref("verbose-cli-output")],
            &rejected_oid,
        )?;
        let neutral_snapshot = store_seed_bundle(
            object_store,
            repo_path,
            "req_demo_neutral",
            &[&canonical_request_ref("cache-observability-note")],
            &neutral_oid,
        )?;
        let gallery = vec![
            SeedRequest {
                id: "req_demo_working",
                name: "request-queue-copy",
                title: "Tighten request queue copy",
                base_oid: main_oid.clone(),
                head_oid: working_oid,
                snapshot: working_snapshot,
                description_markdown: Some("A private working draft for the request author."),
                revisions: Vec::new(),
                outcome: SeedRequestOutcome::Draft,
                audience: RequestAudience::Public,
                now_unix: 1_800_000_050,
            },
            SeedRequest {
                id: "req_demo_ready",
                name: "bounded-retry-timing",
                title: "Add bounded retry timing",
                base_oid: main_oid.clone(),
                head_oid: ready_oid,
                snapshot: ready_snapshot,
                description_markdown: Some(request_discussions::READY_REQUEST_DESCRIPTION),
                revisions: ready_revisions,
                outcome: SeedRequestOutcome::Open,
                audience: RequestAudience::Public,
                now_unix: 1_800_000_100,
            },
            SeedRequest {
                id: "req_demo_held",
                name: "remote-troubleshooting",
                title: "Add remote troubleshooting",
                base_oid: main_oid.clone(),
                head_oid: held_oid,
                snapshot: held_snapshot,
                description_markdown: None,
                revisions: Vec::new(),
                outcome: SeedRequestOutcome::Open,
                audience: RequestAudience::Private,
                now_unix: 1_800_000_200,
            },
            SeedRequest {
                id: "req_demo_accepted",
                name: "document-release-flow",
                title: "Document the release flow",
                base_oid: initial_oid,
                head_oid: main_oid.clone(),
                snapshot: accepted_snapshot,
                description_markdown: None,
                revisions: Vec::new(),
                outcome: SeedRequestOutcome::Merged,
                audience: RequestAudience::Private,
                now_unix: 1_800_000_300,
            },
            SeedRequest {
                id: "req_demo_rejected",
                name: "verbose-cli-output",
                title: "Try verbose CLI output",
                base_oid: main_oid.clone(),
                head_oid: rejected_oid,
                snapshot: rejected_snapshot,
                description_markdown: None,
                revisions: Vec::new(),
                outcome: SeedRequestOutcome::Closed,
                audience: RequestAudience::Private,
                now_unix: 1_800_000_400,
            },
            SeedRequest {
                id: "req_demo_neutral",
                name: "cache-observability-note",
                title: "Document the cache tradeoff",
                base_oid: main_oid,
                head_oid: neutral_oid,
                snapshot: neutral_snapshot,
                description_markdown: Some("A public request kept as closed history."),
                revisions: Vec::new(),
                outcome: SeedRequestOutcome::Closed,
                audience: RequestAudience::Public,
                now_unix: 1_800_000_500,
            },
        ];
        Ok((main_head, main_pack_span, gallery, segment_upload))
    })
}

pub(super) fn seed_request_branch(
    repo_path: &FsPath,
    request_id: &str,
    commit: SeedGitCommit<'_>,
    main_oid: &str,
) -> Result<String, ApiError> {
    apply_seed_commits(repo_path, &[commit])?;
    let head_oid = seed_git_head(repo_path)?;
    let request_ref = canonical_request_ref(request_id);
    seed_git(
        Some(repo_path),
        &["update-ref", &request_ref, &head_oid],
        "creating seeded request ref",
    )?;
    seed_git(
        Some(repo_path),
        &["reset", "--hard", main_oid],
        "restoring seeded main branch",
    )?;
    Ok(head_oid)
}

pub(super) fn with_seed_git_repo<T>(
    label: &str,
    build: impl FnOnce(&FsPath) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    let repo_path = temp_seed_git_repo_path(label)?;
    if repo_path.exists() {
        fs::remove_dir_all(&repo_path).map_err(ApiError::internal)?;
    }
    fs::create_dir_all(&repo_path).map_err(ApiError::internal)?;

    let result = (|| {
        seed_git(
            None,
            &["init", repo_path.to_string_lossy().as_ref()],
            "initializing seeded Git repo",
        )?;
        seed_git(
            Some(&repo_path),
            &["checkout", "-B", DEFAULT_GIT_BRANCH],
            "creating seeded default branch",
        )?;
        build(&repo_path)
    })();

    let cleanup = fs::remove_dir_all(&repo_path);
    if let Err(error) = cleanup
        && result.is_ok()
    {
        return Err(ApiError::internal(error));
    }
    result
}

pub(super) fn apply_seed_commits(
    repo_path: &FsPath,
    commits: &[SeedGitCommit<'_>],
) -> Result<(), ApiError> {
    for commit in commits {
        for (path, content) in commit.files {
            let path = repo_path.join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(ApiError::internal)?;
            }
            fs::write(path, content).map_err(ApiError::internal)?;
        }
        seed_git(Some(repo_path), &["add", "--all"], "adding seeded files")?;
        seed_git(
            Some(repo_path),
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--no-gpg-sign",
                "--no-verify",
                "--message",
                commit.message,
            ],
            "committing seeded files",
        )?;
    }
    Ok(())
}

pub(super) fn store_seed_bundle(
    object_store: &dyn ObjectStore,
    repo_path: &FsPath,
    label: &str,
    refs: &[&str],
    head_oid: &str,
) -> Result<SourceBlob, ApiError> {
    let bundle_path = repo_path.join(format!("{label}.bundle"));
    let bundle = bundle_path.to_string_lossy().to_string();
    let mut args = vec!["bundle", "create", bundle.as_str()];
    args.extend_from_slice(refs);
    seed_git(Some(repo_path), &args, "creating seeded Git bundle")?;
    let bytes = fs::read(&bundle_path).map_err(ApiError::internal)?;
    fs::remove_file(&bundle_path).map_err(ApiError::internal)?;
    let mut snapshot = put_content_object(object_store, ContentObjectKind::GitBundle, bytes)?;
    snapshot.git_oid = head_oid.to_string();
    Ok(snapshot)
}

pub(super) fn seed_git_head(repo_path: &FsPath) -> Result<String, ApiError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| {
            ApiError::infrastructure_unavailable(format!("reading seeded head: {error}"))
        })?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading seeded head: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(ApiError::internal)?
        .trim()
        .to_string())
}

pub(super) fn temp_seed_git_repo_path(label: &str) -> Result<std::path::PathBuf, ApiError> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(ApiError::internal)?
        .as_nanos();
    let sequence = SEED_TEMP_REPO_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(std::env::temp_dir().join(format!(
        "scope-vcs-dev-seed-{}-{}-{nanos}-{sequence}",
        std::process::id(),
        label
    )))
}

pub(super) fn seed_git(repo: Option<&FsPath>, args: &[&str], action: &str) -> Result<(), ApiError> {
    let mut command = Command::new("git");
    if let Some(repo) = repo {
        command.arg("-C").arg(repo);
    }
    let output = command
        .args(args)
        .env("GIT_AUTHOR_NAME", "Scope Dev Seed")
        .env("GIT_AUTHOR_EMAIL", "scope-dev@example.invalid")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_NAME", "Scope Dev Seed")
        .env("GIT_COMMITTER_EMAIL", "scope-dev@example.invalid")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .output()
        .map_err(|error| {
            ApiError::infrastructure_unavailable(format!("failed {action}: {error}"))
        })?;
    if output.status.success() {
        return Ok(());
    }

    Err(ApiError::infrastructure_unavailable(format!(
        "{action}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}
