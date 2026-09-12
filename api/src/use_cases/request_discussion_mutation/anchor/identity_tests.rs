use super::{commit_paths, visible_commit_paths};
use axum::http::StatusCode;
use scope_domain::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::ContentRef,
    policy::{Policy, ScopePath, Visibility, VisibilityRule},
    repository::access::RepositoryAccess,
    requests::RequestRevision,
};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

#[test]
fn commit_paths_accepts_large_identity_output_and_uses_the_first_parent() {
    let fixture = fixture();
    let parents = git(
        fixture.directory.path(),
        &["show", "-s", "--format=%P", &fixture.commit],
        None,
    );
    assert!(parents.len() > 64 * 1024);

    let (paths, hidden) = commit_paths(
        fixture.directory.path(),
        &Policy::new(Visibility::Public),
        RepositoryAccess::public(),
        &fixture.commit,
    )
    .unwrap();

    assert!(!hidden);
    assert_eq!(
        paths
            .into_iter()
            .map(|path| path.to_string())
            .collect::<Vec<_>>(),
        vec!["/public.txt"]
    );
}

#[test]
fn commit_paths_rejects_a_root_commit_with_the_existing_conflict() {
    let fixture = fixture();

    let error = commit_paths(
        fixture.directory.path(),
        &Policy::new(Visibility::Public),
        RepositoryAccess::public(),
        &fixture.base,
    )
    .err()
    .unwrap();

    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(
        error.public_message(),
        "request revision commit must have a parent"
    );
}

#[test]
fn visible_commit_paths_requires_revision_membership_and_full_visibility() {
    let fixture = fixture();
    let revision = RequestRevision {
        id: "revision-1".to_string(),
        request_id: "request-1".to_string(),
        position: 1,
        actor_user_id: "owner-1".to_string(),
        old_head_oid: fixture.base.clone(),
        new_head_oid: fixture.commit.clone(),
        git_snapshot: SourceBlob {
            content_ref: ContentRef::blob_sha256("snapshot"),
            sha256: "snapshot".to_string(),
            git_oid: "snapshot".to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 1,
        },
        created_at_unix: 1,
    };
    let public_policy = Policy::new(Visibility::Public);

    let paths = visible_commit_paths(
        fixture.directory.path(),
        &public_policy,
        RepositoryAccess::public(),
        &revision,
        &fixture.commit,
    )
    .unwrap();
    assert_eq!(
        paths
            .into_iter()
            .map(|path| path.to_string())
            .collect::<Vec<_>>(),
        vec!["/public.txt"]
    );

    assert_not_found(
        visible_commit_paths(
            fixture.directory.path(),
            &public_policy,
            RepositoryAccess::public(),
            &revision,
            &fixture.outsider,
        )
        .err()
        .unwrap(),
    );

    let mut private_policy = Policy::new(Visibility::Public);
    private_policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/public.txt").unwrap(),
        ))
        .unwrap();
    assert_not_found(
        visible_commit_paths(
            fixture.directory.path(),
            &private_policy,
            RepositoryAccess::public(),
            &revision,
            &fixture.commit,
        )
        .err()
        .unwrap(),
    );
}

fn assert_not_found(error: crate::error::ApiError) {
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    assert_eq!(error.public_message(), "request revision commit not found");
}

struct Fixture {
    directory: tempfile::TempDir,
    base: String,
    commit: String,
    outsider: String,
}

fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "--quiet"], None);
    let empty_tree = git(directory.path(), &["mktree"], Some(""));
    let base = git(
        directory.path(),
        &["commit-tree", &empty_tree, "-m", "base"],
        None,
    );
    let blob = git(
        directory.path(),
        &["hash-object", "-w", "--stdin"],
        Some("public\n"),
    );
    let changed_tree = git(
        directory.path(),
        &["mktree"],
        Some(&format!("100644 blob {blob}\tpublic.txt\n")),
    );
    let matching_parent = git(
        directory.path(),
        &["commit-tree", &changed_tree, "-p", &base, "-m", "matching"],
        None,
    );
    let mut raw_commit = format!("tree {changed_tree}\nparent {base}\n");
    for _ in 1..1_600 {
        raw_commit.push_str(&format!("parent {matching_parent}\n"));
    }
    raw_commit.push_str(
        "author Scope Test <scope@example.test> 1700000000 +0000\n\
         committer Scope Test <scope@example.test> 1700000000 +0000\n\
         \nlarge identity\n",
    );
    let commit = git(
        directory.path(),
        &["hash-object", "-t", "commit", "-w", "--stdin"],
        Some(&raw_commit),
    );
    let outsider = git(
        directory.path(),
        &["commit-tree", &empty_tree, "-m", "outsider"],
        None,
    );
    Fixture {
        directory,
        base,
        commit,
        outsider,
    }
}

fn git(repo: &Path, args: &[&str], stdin: Option<&str>) -> String {
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
