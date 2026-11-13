use std::{
    path::Path,
    process::{Command, Output},
};

pub(crate) fn run_git<const N: usize>(cwd: &Path, args: [&str; N]) -> Output {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub(crate) fn commit_all(cwd: &Path, message: &str) -> String {
    run_git(cwd, ["add", "-A"]);
    run_git(
        cwd,
        [
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@example.test",
            "commit",
            "-qm",
            message,
        ],
    );
    String::from_utf8(run_git(cwd, ["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned()
}
