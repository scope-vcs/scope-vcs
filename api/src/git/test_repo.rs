use crate::git::command::{git_stdout_text, run_git};
use std::{
    io::Write as _,
    ops::Deref,
    path::Path,
    process::{Command, Stdio},
};
use tempfile::TempDir;

pub(crate) struct TestRepo(TempDir);

impl TestRepo {
    pub(crate) fn init() -> Self {
        let dir = tempfile::tempdir().unwrap();
        run_git(
            None,
            &[
                "init",
                "--initial-branch=main",
                dir.path().to_string_lossy().as_ref(),
            ],
            "initializing test repository",
        )
        .unwrap();
        for (key, value) in [("user.name", "Test"), ("user.email", "test@scope.local")] {
            run_git(
                Some(dir.path()),
                &["config", key, value],
                "configuring test identity",
            )
            .unwrap();
        }
        Self(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        self.0.path()
    }

    pub(crate) fn commit_all(&self, message: &str) {
        run_git(Some(self.path()), &["add", "."], "staging test files").unwrap();
        run_git(
            Some(self.path()),
            &["commit", "-m", message],
            "committing test files",
        )
        .unwrap();
    }

    pub(crate) fn oid(&self, revision: &str) -> String {
        git_stdout_text(self.path(), &["rev-parse", revision], "reading test oid")
            .unwrap()
            .trim()
            .to_string()
    }
}

impl Deref for TestRepo {
    type Target = Path;

    fn deref(&self) -> &Path {
        self.path()
    }
}

pub(crate) fn pinned_identity_git(repo: &Path, args: &[&str], stdin: Option<&str>) -> String {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Scope Test")
        .env("GIT_AUTHOR_EMAIL", "scope@example.test")
        .env("GIT_AUTHOR_DATE", "1700000000 +0000")
        .env("GIT_COMMITTER_NAME", "Scope Test")
        .env("GIT_COMMITTER_EMAIL", "scope@example.test")
        .env("GIT_COMMITTER_DATE", "1700000000 +0000")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}
