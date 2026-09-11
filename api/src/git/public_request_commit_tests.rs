use std::{fs, path::Path, process::Command};

use scope_domain::{content_ref::ContentRef, policy::ScopePath};

use super::*;

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Original Author")
        .env("GIT_AUTHOR_EMAIL", "original@example.test")
        .env("GIT_COMMITTER_NAME", "Different Committer")
        .env("GIT_COMMITTER_EMAIL", "committer@example.test")
        .env("GIT_AUTHOR_DATE", "2001-02-03T04:05:06+00:00")
        .env("GIT_COMMITTER_DATE", "2002-03-04T05:06:07+00:00")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_string()
}

fn init_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "--quiet"]);
    repo
}

fn native(repo: &Path, oid: String, changed_paths: &[&str]) -> NativePublicCommit {
    NativePublicCommit {
        tree_oid: git(repo, &["show", "-s", "--format=%T", &oid]),
        parent_oids: git(repo, &["show", "-s", "--format=%P", &oid])
            .split_ascii_whitespace()
            .map(str::to_string)
            .collect(),
        oid,
        changed_paths: changed_paths
            .iter()
            .map(|path| ScopePath::parse(format!("/{path}")).unwrap())
            .collect(),
    }
}

fn blob_oid(repo: &Path, commit: &str, path: &str) -> String {
    git(repo, &["rev-parse", &format!("{commit}:{path}")])
}

#[test]
fn reads_original_author_time_message_and_root_blob() {
    let repo = init_repo();
    fs::write(repo.path().join("root.txt"), b"root content\n").unwrap();
    git(repo.path(), &["add", "."]);
    let tree = git(repo.path(), &["write-tree"]);
    let oid = git(
        repo.path(),
        &["commit-tree", &tree, "-m", "  leading subject\n\nbody  "],
    );
    let commit = native(repo.path(), oid.clone(), &["root.txt"]);
    let details = inspect_native_public_commit(repo.path(), &commit).unwrap();
    assert_eq!(details.author, "Original Author <original@example.test>");
    assert_eq!(details.message, "  leading subject\n\nbody  ");
    assert_eq!(details.occurred_at_unix, 1_015_218_367);
    assert_eq!(details.changes.len(), 1);
    let change = &details.changes[0];
    assert!(change.old_content.is_none());
    assert_eq!(change.visibility, Visibility::Public);
    let blob = change.new_content.as_ref().unwrap();
    let expected_oid = blob_oid(repo.path(), &oid, "root.txt");
    assert_eq!(blob.git_oid, expected_oid);
    assert_eq!(blob.content_ref, ContentRef::git_blob(&expected_oid));
    assert_eq!(blob.sha256, expected_oid);
    assert_eq!(blob.git_file_mode, "100644");
    assert_eq!(blob.size_bytes, 13);
}

#[test]
fn merge_uses_first_parent_blobs_independently_of_safety_paths() {
    let repo = init_repo();
    for (path, text) in [
        ("edit.txt", "before\n"),
        ("gone.txt", "deleted\n"),
        ("tool.sh", "echo ok\n"),
    ] {
        fs::write(repo.path().join(path), text).unwrap();
    }
    git(repo.path(), &["add", "."]);
    let tree = git(repo.path(), &["write-tree"]);
    let first = git(repo.path(), &["commit-tree", &tree, "-m", "first"]);
    fs::write(repo.path().join("side.txt"), "side\n").unwrap();
    git(repo.path(), &["add", "."]);
    let side_tree = git(repo.path(), &["write-tree"]);
    let side = git(
        repo.path(),
        &["commit-tree", &side_tree, "-p", &first, "-m", "side"],
    );
    fs::write(repo.path().join("edit.txt"), "after\n").unwrap();
    fs::remove_file(repo.path().join("gone.txt")).unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["update-index", "--chmod=+x", "tool.sh"]);
    let merge_tree = git(repo.path(), &["write-tree"]);
    let merge = git(
        repo.path(),
        &[
            "commit-tree",
            &merge_tree,
            "-p",
            &first,
            "-p",
            &side,
            "-m",
            "merge",
        ],
    );
    let commit = native(
        repo.path(),
        merge.clone(),
        &["edit.txt", "gone.txt", "tool.sh"],
    );
    assert_eq!(commit.parent_oids, vec![first.clone(), side]);
    let details = inspect_native_public_commit(repo.path(), &commit).unwrap();
    assert_eq!(details.changes.len(), 4);
    let changes = details
        .changes
        .iter()
        .map(|change| (change.path.as_str(), change))
        .collect::<BTreeMap<_, _>>();
    let edit = changes["/edit.txt"];
    assert_eq!(
        edit.old_content.as_ref().unwrap().git_oid,
        blob_oid(repo.path(), &first, "edit.txt")
    );
    assert_eq!(
        edit.new_content.as_ref().unwrap().git_oid,
        blob_oid(repo.path(), &merge, "edit.txt")
    );
    let deletion = changes["/gone.txt"];
    let deleted_blob = deletion.old_content.as_ref().unwrap();
    assert_eq!(
        deleted_blob.git_oid,
        blob_oid(repo.path(), &first, "gone.txt")
    );
    assert_eq!(deleted_blob.size_bytes, 8);
    assert!(deletion.new_content.is_none());
    let executable = changes["/tool.sh"];
    assert_eq!(
        executable.old_content.as_ref().unwrap().git_file_mode,
        "100644"
    );
    assert_eq!(
        executable.new_content.as_ref().unwrap().git_file_mode,
        "100755"
    );
    assert_eq!(
        executable.old_content.as_ref().unwrap().git_oid,
        executable.new_content.as_ref().unwrap().git_oid
    );
    assert!(changes["/side.txt"].old_content.is_none());
    assert_eq!(
        changes["/side.txt"]
            .new_content
            .as_ref()
            .unwrap()
            .size_bytes,
        5
    );
    assert!(
        details
            .changes
            .iter()
            .all(|change| change.visibility == Visibility::Public)
    );
}

#[test]
fn rejects_changed_recorded_tree_parents_or_commit_identity() {
    let repo = init_repo();
    let tree = git(repo.path(), &["write-tree"]);
    let oid = git(repo.path(), &["commit-tree", &tree, "-m", "empty"]);
    let commit = native(repo.path(), oid.clone(), &[]);
    assert!(
        inspect_native_public_commit(repo.path(), &commit)
            .unwrap()
            .changes
            .is_empty()
    );
    let mut wrong_tree = commit.clone();
    wrong_tree.tree_oid = "0".repeat(40);
    assert!(inspect_native_public_commit(repo.path(), &wrong_tree).is_err());
    let mut wrong_parents = commit.clone();
    wrong_parents.parent_oids.push(oid.clone());
    assert!(inspect_native_public_commit(repo.path(), &wrong_parents).is_err());
    let mut abbreviated = commit;
    abbreviated.oid.truncate(12);
    assert!(inspect_native_public_commit(repo.path(), &abbreviated).is_err());
}

#[test]
fn reads_non_utf8_author_and_message_from_native_git_object() {
    let repo = init_repo();
    let tree = git(repo.path(), &["write-tree"]);
    let mut object = format!("tree {tree}\nauthor Original ").into_bytes();
    object.extend_from_slice(b"\xff <original@example.test> 1700000000 +0000\ncommitter Committer <committer@example.test> 1700000001 +0000\n\n  Subject \xfe\n\nbody  \n");
    fs::write(repo.path().join("commit-object"), object).unwrap();
    let oid = git(
        repo.path(),
        &["hash-object", "-t", "commit", "-w", "commit-object"],
    );
    let commit = native(repo.path(), oid, &[]);
    let details = inspect_native_public_commit(repo.path(), &commit).unwrap();
    assert_eq!(details.author, "Original � <original@example.test>");
    assert_eq!(details.message, "  Subject �\n\nbody  ");
    assert_eq!(details.occurred_at_unix, 1_700_000_001);
    assert!(details.changes.is_empty());
}
