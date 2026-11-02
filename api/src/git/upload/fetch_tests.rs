use super::*;
use crate::git::command::{git_stdout_text, run_git};

fn folder() -> GitRepoHandle {
    let directory = tempfile::tempdir().unwrap();
    let repo = GitRepoHandle::temporary(directory, Vec::new()).unwrap();
    run_git(
        None,
        &["init", "--bare", "--quiet", repo.to_str().unwrap()],
        "init fetch fixture",
    )
    .unwrap();
    let blob = git_command_output(
        Command::new("git")
            .arg("-C")
            .arg(repo.as_ref())
            .args(["hash-object", "-w", "--stdin"]),
        Some(b"fetch fixture\n"),
    )
    .unwrap();
    let tree = git_command_output(
        Command::new("git")
            .arg("-C")
            .arg(repo.as_ref())
            .arg("mktree"),
        Some(
            format!(
                "100644 blob {}\tfile.txt\n",
                String::from_utf8_lossy(&blob).trim()
            )
            .as_bytes(),
        ),
    )
    .unwrap();
    let commit = git_command_output(
        Command::new("git")
            .arg("-C")
            .arg(repo.as_ref())
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "commit-tree",
            ])
            .arg(String::from_utf8_lossy(&tree).trim())
            .args(["-m", "fixture"]),
        None,
    )
    .unwrap();
    run_git(
        Some(&repo),
        &[
            "update-ref",
            "refs/heads/main",
            String::from_utf8_lossy(&commit).trim(),
        ],
        "set fixture main",
    )
    .unwrap();
    std::fs::write(repo.join("objects/info/alternates"), "").unwrap();
    repo
}

#[tokio::test]
async fn fetch_folder_is_removed_after_stream_success_error_and_disconnect() {
    let budgets = RuntimeBudgets::from_config(Default::default());
    for outcome in ["success", "invalid-want", "disconnect"] {
        let repo = folder();
        let path = repo.as_ref().to_path_buf();
        let head = git_stdout_text(
            &repo,
            &["rev-parse", "refs/heads/main"],
            "read fetch fixture",
        )
        .unwrap();
        let head = if outcome == "invalid-want" {
            "0000000000000000000000000000000000000001"
        } else {
            head.trim()
        };
        let want = format!("want {head}\n");
        let request = format!("{:04x}{want}00000009done\n", want.len() + 4);
        let response = git_upload_pack_response(
            repo,
            request.as_bytes(),
            Duration::from_secs(5),
            budgets.admit_upload_pack().await.unwrap(),
        )
        .await;
        match outcome {
            "invalid-want" => {
                assert_eq!(response.unwrap_err().status(), StatusCode::BAD_REQUEST);
            }
            "success" => {
                let body = axum::body::to_bytes(response.unwrap().into_body(), usize::MAX)
                    .await
                    .unwrap();
                assert!(body.windows(4).any(|bytes| bytes == b"PACK"));
            }
            _ => drop(response.unwrap()),
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while path.exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
