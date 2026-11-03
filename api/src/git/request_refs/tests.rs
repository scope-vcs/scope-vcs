use super::*;
use crate::git::test_repo::TestRepo;

#[test]
fn request_names_come_from_top_level_branches_other_than_main() {
    assert_eq!(
        request_name_from_ref("refs/heads/railway-upload"),
        Some("railway-upload")
    );
    for refname in [
        "refs/heads/main",
        "refs/heads/requests/nested",
        "refs/tags/railway-upload",
    ] {
        assert_eq!(request_name_from_ref(refname), None);
    }
}

#[test]
fn request_ref_diff_rejects_invalid_request_names_before_lookup() {
    for name in ["head", "scope", "UPPER-CASE"] {
        let after = vec![(
            format!("refs/heads/{name}"),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        )];
        let error = request_ref_update_from_refs(&[], &after).unwrap_err();
        assert!(error.public_message().contains("invalid request branch"));
    }
}

#[test]
fn request_ref_diff_rejects_delete_and_tracks_name() {
    let before = vec![(
        "refs/heads/railway-upload".to_string(),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
    )];
    let after = vec![(
        "refs/heads/railway-upload".to_string(),
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
    )];
    let update = request_ref_update_from_refs(&before, &after)
        .unwrap()
        .unwrap();
    assert_eq!(update.request_name, "railway-upload");
    assert_eq!(update.request_ref, "refs/heads/railway-upload");

    let error = request_ref_update_from_refs(&before, &[]).unwrap_err();
    assert!(error.public_message().contains("deletes"));
}

#[test]
fn request_ref_head_must_be_commit_object_not_annotated_tag() {
    let repo = TestRepo::init();
    run_git(
        Some(&repo),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
        "create test commit",
    )
    .unwrap();
    run_git(
        Some(&repo),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@example.com",
            "tag",
            "-a",
            "request-tag",
            "-m",
            "tag",
        ],
        "create annotated tag",
    )
    .unwrap();
    let head = git_stdout(&repo, &["rev-parse", "HEAD"]);
    let tag = git_stdout(&repo, &["rev-parse", "request-tag^{tag}"]);

    ensure_request_ref_oid_is_commit(&repo, head.trim()).unwrap();
    let error = ensure_request_ref_oid_is_commit(&repo, tag.trim()).unwrap_err();
    assert!(error.public_message().contains("must point at commits"));
}

#[test]
fn main_import_ref_listing_ignores_seeded_named_requests() {
    let repo = TestRepo::init();
    run_git(
        Some(&repo),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
        "create main commit",
    )
    .unwrap();
    let head = git_stdout(&repo, &["rev-parse", "HEAD"]);
    run_git(
        Some(&repo),
        &["branch", "railway-upload", head.trim()],
        "seed named request",
    )
    .unwrap();

    let refs = crate::git::import::git_refs(&repo).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].0, "refs/heads/main");
}

fn git_stdout(repo: &FsPath, args: &[&str]) -> String {
    let output = run_git_output(Some(repo), args, "read test git stdout").unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}
