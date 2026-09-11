use super::*;
use std::{
    fs,
    process::{Command, Stdio},
};

#[test]
fn lock_exclusion_depends_on_owner_not_timestamp_and_survives_reopening() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("request.lock");
    fs::write(&path, "pid=1\ncreated_at_unix=1\n").unwrap();
    let first = acquire_git_lock(&path, "busy", Duration::ZERO).unwrap();
    fs::write(
        &path,
        format!("pid={}\ncreated_at_unix=1\n", std::process::id()),
    )
    .unwrap();
    assert!(acquire_git_lock(&path, "busy", Duration::ZERO).is_err());
    drop(first);
    let second = acquire_git_lock(&path, "busy", Duration::ZERO).unwrap();
    assert!(acquire_git_lock(&path, "busy", Duration::ZERO).is_err());
    drop(second);
    assert!(path.is_file());
}

#[test]
fn child_lock_holder() {
    let Some(path) = std::env::var_os("SCOPE_TEST_REQUEST_LOCK") else {
        return;
    };
    let path = PathBuf::from(path);
    let _lock = acquire_git_lock(&path, "busy", Duration::ZERO).unwrap();
    fs::write(path.with_extension("ready"), "ready").unwrap();
    loop {
        thread::park();
    }
}

#[test]
fn process_death_releases_lock_without_recovery_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("request.lock");
    let ready = path.with_extension("ready");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "git::request_refs::locks::tests::child_lock_holder",
            "--nocapture",
        ])
        .env("SCOPE_TEST_REQUEST_LOCK", &path)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(LOCK_RETRY);
    }
    let acquired = ready.exists();
    let excluded = acquired && acquire_git_lock(&path, "busy", Duration::ZERO).is_err();
    let _ = child.kill();
    child.wait().unwrap();
    assert!(acquired, "child did not acquire the lock");
    assert!(excluded, "second process acquired an owned lock");
    drop(acquire_git_lock(&path, "busy", Duration::ZERO).unwrap());
}

#[tokio::test(flavor = "current_thread")]
async fn contended_request_update_lock_does_not_block_runtime_progress() {
    let state = AppState::test_state();
    let incarnation = RepositoryIncarnation::new("owner/repo", "repoi_lock_progress").unwrap();
    let request_ref = "refs/heads/work";
    let path = request_ref_update_lock_path(&state, &incarnation, request_ref);
    let first = acquire_git_lock(&path, "busy", Duration::ZERO).unwrap();
    let contender = tokio::spawn(async move {
        acquire_request_ref_update_lock_async(&state, &incarnation, request_ref).await
    });
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(!contender.is_finished());
    drop(first);
    drop(
        tokio::time::timeout(Duration::from_secs(2), contender)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
    );
}
