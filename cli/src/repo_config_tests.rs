use super::*;
use crate::test_support::TempDir;

#[cfg(unix)]
#[test]
fn write_and_create_reject_symlinked_state_paths() {
    use std::os::unix::fs::symlink;

    for (operation, symlink_directory) in [
        ("write", false),
        ("create", false),
        ("write", true),
        ("create", true),
    ] {
        let dir = TempDir::git_repo(
            &format!(
                "{operation}-{}-symlink",
                if symlink_directory { "dir" } else { "file" }
            ),
            "main",
        );
        let paths = repo_state_paths(&dir.path).unwrap();
        let outside = dir.path.join("outside");
        if symlink_directory {
            fs::create_dir(&outside).unwrap();
            symlink(&outside, &paths.directory).unwrap();
        } else {
            fs::create_dir(&paths.directory).unwrap();
            fs::write(&outside, default_repo_config_json()).unwrap();
            symlink(&outside, &paths.config).unwrap();
        }

        let error = if operation == "write" {
            write_worktree_scope_repo_config(&dir.path, &default_scope_repo_config()).unwrap_err()
        } else {
            ensure_scope_repo_config_exists(&dir.path).unwrap_err()
        };
        assert!(error.to_string().contains(if symlink_directory {
            "Scope repo state directory cannot be a symlink"
        } else {
            "Scope repo config cannot be a symlink"
        }));
    }
}

#[test]
fn synced_config_lives_only_in_per_worktree_git_state() {
    let dir = TempDir::git_repo("state", "main");
    let config = default_scope_repo_config();

    write_worktree_scope_repo_config_with_base(&dir.path, &config).unwrap();
    let paths = repo_state_paths(&dir.path).unwrap();

    assert_eq!(
        load_worktree_scope_repo_config_base_hash(&dir.path).unwrap(),
        repo_config_fingerprint(&config).unwrap()
    );
    assert_eq!(repo_config_path(&dir.path).unwrap(), paths.config);
    assert!(paths.config.is_file());
    assert!(paths.state.is_file());
    assert!(!dir.path.join(".scope").exists());
    assert!(!dir.path.join(".gitignore").exists());
    assert!(
        !fs::read_to_string(dir.path.join(".git/info/exclude"))
            .unwrap_or_default()
            .lines()
            .any(|line| line.trim() == "/.scope/")
    );
}

#[test]
fn linked_worktrees_get_distinct_scope_state_directories() {
    let main = TempDir::git_repo("linked-main", "main");
    fs::write(main.path.join("README.md"), "initial\n").unwrap();
    main.run_git(["add", "README.md"]);
    main.run_git([
        "-c",
        "user.email=scope@example.test",
        "-c",
        "user.name=Scope Test",
        "commit",
        "-m",
        "initial",
    ]);
    let linked = main.path.join("linked");
    main.run_git(["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);

    ensure_scope_repo_config_exists(&main.path).unwrap();
    ensure_scope_repo_config_exists(&linked).unwrap();
    let main_path = repo_config_path(&main.path).unwrap();
    let linked_path = repo_config_path(&linked).unwrap();

    assert_ne!(main_path, linked_path);
    assert!(main_path.ends_with(".git/scope/repo.json"));
    assert!(linked_path.ends_with("worktrees/linked/scope/repo.json"));
    assert!(!main.path.join(".scope").exists());
    assert!(!linked.join(".scope").exists());
    assert!(is_linked_worktree(&linked).unwrap());
    assert!(!is_linked_worktree(&main.path).unwrap());
}

#[test]
fn server_config_initializes_only_absent_worktree_state() {
    let dir = TempDir::git_repo("sync-missing-state", "main");
    let mut server = default_scope_repo_config();
    server.visibility.default = ConfigVisibility::Public;
    assert_eq!(
        worktree_scope_repo_config_presence(&dir.path).unwrap(),
        WorktreeRepoConfigPresence::Absent
    );
    assert_eq!(
        sync_missing_worktree_scope_repo_config(&dir.path, &server).unwrap(),
        WorktreeRepoConfigSync::Created
    );
    assert_eq!(load_worktree_scope_repo_config(&dir.path).unwrap(), server);
    assert_eq!(
        load_worktree_scope_repo_config_base_hash(&dir.path).unwrap(),
        repo_config_fingerprint(&server).unwrap()
    );

    let local = default_scope_repo_config();
    write_worktree_scope_repo_config(&dir.path, &local).unwrap();
    assert_eq!(
        sync_missing_worktree_scope_repo_config(&dir.path, &server).unwrap(),
        WorktreeRepoConfigSync::Unchanged
    );
    assert_eq!(load_worktree_scope_repo_config(&dir.path).unwrap(), local);
    assert_eq!(
        load_worktree_scope_repo_config_base_hash(&dir.path).unwrap(),
        repo_config_fingerprint(&server).unwrap()
    );
}

#[test]
fn partial_worktree_state_recovers_only_when_local_matches_server() {
    let dir = TempDir::git_repo("partial-state", "main");
    let mut server = default_scope_repo_config();
    server.visibility.default = ConfigVisibility::Public;
    write_worktree_scope_repo_config(&dir.path, &server).unwrap();
    assert_eq!(
        sync_missing_worktree_scope_repo_config(&dir.path, &server).unwrap(),
        WorktreeRepoConfigSync::BaseRecovered
    );
    assert_eq!(
        load_worktree_scope_repo_config_base_hash(&dir.path).unwrap(),
        repo_config_fingerprint(&server).unwrap()
    );

    fs::remove_file(repo_state_paths(&dir.path).unwrap().state).unwrap();
    let local = default_scope_repo_config();
    write_worktree_scope_repo_config(&dir.path, &local).unwrap();
    let error = sync_missing_worktree_scope_repo_config(&dir.path, &server).unwrap_err();
    assert!(error.to_string().contains("edits but no sync base"));
    assert_eq!(load_worktree_scope_repo_config(&dir.path).unwrap(), local);
    assert!(!repo_state_paths(&dir.path).unwrap().state.exists());
}

#[test]
fn invalid_partial_worktree_state_is_preserved() {
    let dir = TempDir::git_repo("invalid-partial-state", "main");
    let paths = repo_state_paths(&dir.path).unwrap();
    fs::create_dir(&paths.directory).unwrap();
    fs::write(&paths.config, "not JSON").unwrap();
    let error = sync_missing_worktree_scope_repo_config(&dir.path, &default_scope_repo_config())
        .unwrap_err();
    assert!(error.to_string().contains("parse Scope repo config"));
    assert_eq!(fs::read_to_string(&paths.config).unwrap(), "not JSON");
    assert!(!paths.state.exists());

    fs::remove_file(&paths.config).unwrap();
    fs::write(&paths.state, "not JSON").unwrap();
    let error = sync_missing_worktree_scope_repo_config(&dir.path, &default_scope_repo_config())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("without a local visibility config")
    );
    assert!(!paths.config.exists());
}
