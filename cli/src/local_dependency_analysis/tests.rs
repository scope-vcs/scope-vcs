use super::*;
use crate::test_support::TempDir;
use scope_domain::{
    dependency_analysis::{
        DependencyEdge, DependencyEdgeKind, DependencyGap, evaluate_dependency_analysis,
    },
    repo_config::{ConfigVisibility, RepoConfig, RepoConfigVisibilityRule},
};
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

fn repository() -> (TempDir, String) {
    let repo = TempDir::git_repo("dependency-snapshot", "main");
    fs::create_dir_all(repo.path().join("src")).unwrap();
    fs::create_dir_all(repo.path().join("internal")).unwrap();
    fs::write(
        repo.path().join("src/index.ts"),
        "import '../internal/client';\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("internal/client.ts"),
        "export const secret = 1;\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("tsconfig.json"),
        "{\"compilerOptions\":{\"baseUrl\":\".\"}}",
    )
    .unwrap();
    fs::write(
        repo.path().join("asset.png"),
        b"large asset contents are not analyzer inputs",
    )
    .unwrap();
    repo.run_git(["add", "."]);
    repo.run_git([
        "-c",
        "user.name=Scope Test",
        "-c",
        "user.email=scope@example.test",
        "commit",
        "-qm",
        "snapshot",
    ]);
    let commit = String::from_utf8(repo.run_git(["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned();
    (repo, commit)
}

fn output() -> AnalyzerOutput {
    AnalyzerOutput {
        analyzer_version: DEPENDENCY_ANALYZER_VERSION.into(),
        analyzed_files: vec!["src/index.ts".into(), "internal/client.ts".into()],
        unsupported_files: vec![],
        edges: vec![DependencyEdge {
            source_path: "src/index.ts".into(),
            target_path: "internal/client.ts".into(),
            kind: DependencyEdgeKind::Import,
        }],
        gaps: vec![],
    }
}

#[test]
fn snapshot_reads_committed_bytes_and_resolver_config_without_worktree_edits() {
    let (repo, commit) = repository();
    fs::write(repo.path().join("src/index.ts"), "uncommitted change").unwrap();
    fs::write(repo.path().join("new.ts"), "untracked file").unwrap();
    let token = CancellationToken::new();
    let directory = snapshot::materialize(repo.path(), &commit, &token).unwrap();
    let path = directory.path().to_path_buf();
    assert_eq!(
        fs::read_to_string(path.join("src/index.ts")).unwrap(),
        "import '../internal/client';\n"
    );
    assert!(
        fs::read_to_string(path.join("tsconfig.json"))
            .unwrap()
            .contains("baseUrl")
    );
    assert_eq!(fs::read(path.join("asset.png")).unwrap(), b"");
    assert!(!path.join("new.ts").exists());
    drop(directory);
    assert!(!path.exists());
}

#[test]
fn analysis_reuses_commit_cache_and_visibility_changes_only_reevaluate() {
    let (repo, commit) = repository();
    let calls = AtomicUsize::new(0);
    let token = CancellationToken::new();
    let first = analyze_cached(repo.path(), &commit, &token, |_, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(output())
    })
    .unwrap();
    let second = analyze_cached(repo.path(), &commit, &token, |_, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(output())
    })
    .unwrap();
    assert_eq!(first, second);
    let mut config = RepoConfig::with_default_visibility(ConfigVisibility::Public);
    config.visibility.rules.push(RepoConfigVisibilityRule {
        path: "/internal/**".into(),
        visibility: ConfigVisibility::Private,
    });
    assert_eq!(
        evaluate_dependency_analysis(&second, &config)
            .unwrap()
            .public_file_count,
        1
    );
    config.visibility.rules.clear();
    assert!(
        evaluate_dependency_analysis(&second, &config)
            .unwrap()
            .findings
            .is_empty()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn new_commit_and_wrong_cached_analyzer_version_require_new_analysis() {
    let (repo, commit) = repository();
    let token = CancellationToken::new();
    let first = analyze_cached(repo.path(), &commit, &token, |_, _| Ok(output())).unwrap();
    let cache_path = cache::path(repo.path(), &commit, &token).unwrap();
    let mut stale = first;
    stale.analyzer_version = "old-analyzer".into();
    cache::write(&cache_path, &stale).unwrap();
    let calls = AtomicUsize::new(0);
    analyze_cached(repo.path(), &commit, &token, |_, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(output())
    })
    .unwrap();
    fs::write(
        repo.path().join("src/index.ts"),
        "export const publicValue = 1;",
    )
    .unwrap();
    repo.run_git(["add", "."]);
    repo.run_git([
        "-c",
        "user.name=Scope Test",
        "-c",
        "user.email=scope@example.test",
        "commit",
        "-qm",
        "change imports",
    ]);
    let next = String::from_utf8(repo.run_git(["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned();
    assert_ne!(commit, next);
    let result = analyze_cached(repo.path(), &next, &token, |source, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            fs::read_to_string(source.join("src/index.ts"))
                .unwrap()
                .contains("publicValue")
        );
        let mut output = output();
        output.edges.clear();
        Ok(output)
    })
    .unwrap();
    assert_eq!(result.commit_oid, next);
    assert!(result.edges.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn worktrees_share_analysis_for_the_same_commit() {
    let (repo, commit) = repository();
    let container = TempDir::new("dependency-worktree");
    let worktree = container.path().join("checkout");
    repo.run_git([
        "worktree",
        "add",
        "--detach",
        worktree.to_str().unwrap(),
        &commit,
    ]);
    let token = CancellationToken::new();
    assert_eq!(
        cache::path(repo.path(), &commit, &token).unwrap(),
        cache::path(&worktree, &commit, &token).unwrap()
    );
    analyze_cached(repo.path(), &commit, &token, |_, _| Ok(output())).unwrap();
    analyze_cached(&worktree, &commit, &token, |_, _| {
        panic!("worktree should reuse analysis")
    })
    .unwrap();
}

#[test]
fn mismatched_commit_or_invalid_cached_paths_are_not_reused() {
    let (repo, commit) = repository();
    let token = CancellationToken::new();
    let path = cache::path(repo.path(), &commit, &token).unwrap();
    let mut cached = StoredDependencyAnalysis::from_output(&commit, output()).unwrap();
    cached.commit_oid = "b".repeat(40);
    cache::write(&path, &cached).unwrap();
    assert!(cache::read(&path, &commit).is_none());
    cached.commit_oid = commit.clone();
    cached.edges[0].target_path = "../outside.ts".into();
    cache::write(&path, &cached).unwrap();
    assert!(cache::read(&path, &commit).is_none());
    fs::write(&path, "not JSON").unwrap();
    assert!(cache::read(&path, &commit).is_none());
}

#[test]
fn canceled_analysis_does_not_launch_or_publish_cache() {
    let (repo, commit) = repository();
    let token = CancellationToken::new();
    let path = cache::path(repo.path(), &commit, &token).unwrap();
    token.cancel();
    assert!(
        analyze_cached(repo.path(), &commit, &token, |_, _| panic!(
            "canceled analysis ran"
        ))
        .is_err()
    );
    assert!(!path.exists());
}

#[test]
fn analysis_retains_known_edges_and_coverage_gaps() {
    let (repo, commit) = repository();
    let result = analyze_cached(repo.path(), &commit, &CancellationToken::new(), |_, _| {
        let mut output = output();
        output.gaps.push(DependencyGap {
            path: "src/loader.ts".into(),
            reason: "dynamic import could not be resolved".into(),
        });
        Ok(output)
    })
    .unwrap();
    assert_eq!(result.edges.len(), 1);
    assert_eq!(result.gaps.len(), 1);
}

#[test]
#[ignore = "requires SCOPE_CLI_RUNTIME_DIR pointing to a packaged analyzer"]
fn bundled_analyzer_reports_committed_imports() {
    assert!(
        std::env::var_os("SCOPE_CLI_RUNTIME_DIR").is_some(),
        "set SCOPE_CLI_RUNTIME_DIR to a packaged runtime"
    );
    let (repo, commit) = repository();
    let result = analyze_cached(
        repo.path(),
        &commit,
        &CancellationToken::new(),
        analyze_snapshot,
    )
    .unwrap();
    assert!(
        result
            .edges
            .iter()
            .any(|edge| edge.source_path == "src/index.ts"
                && edge.target_path == "internal/client.ts")
    );
    assert!(result.gaps.is_empty(), "{:?}", result.gaps);
}
