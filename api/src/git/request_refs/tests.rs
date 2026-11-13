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

struct AncestryRepo {
    repo: TestRepo,
    root: String,
    main: String,
    side: String,
    head: String,
    main_tag: String,
}

fn ancestry_repo() -> AncestryRepo {
    let repo = TestRepo::init();
    let commit = |message: &str| {
        run_git(
            Some(&repo),
            &["commit", "--allow-empty", "-m", message],
            "create ancestry commit",
        )
        .unwrap();
        repo.oid("HEAD")
    };
    let root = commit("root");
    let main = commit("main");
    let head = commit("head");
    let side = git_stdout(
        &repo,
        &[
            "commit-tree",
            &format!("{root}^{{tree}}"),
            "-p",
            &root,
            "-m",
            "side",
        ],
    )
    .trim()
    .to_string();
    run_git(
        Some(&repo),
        &["tag", "-a", "main-tag", &main, "-m", "tag"],
        "create annotated main tag",
    )
    .unwrap();
    let main_tag = repo.oid("main-tag");
    AncestryRepo {
        repo,
        root,
        main,
        side,
        head,
        main_tag,
    }
}

#[test]
fn request_head_must_descend_from_both_main_ancestor_and_review_base() {
    let AncestryRepo {
        repo,
        root,
        main,
        side,
        head,
        ..
    } = ancestry_repo();

    ensure_request_ref_descends_from_bases(&repo, &main, &main, &head).unwrap();
    ensure_request_ref_descends_from_bases(&repo, &root, &main, &head).unwrap();
    for (main_ancestor, review_base) in [(&root, &side), (&side, &root), (&side, &side)] {
        let error =
            ensure_request_ref_descends_from_bases(&repo, main_ancestor, review_base, &head)
                .unwrap_err();
        assert!(
            error
                .public_message()
                .contains("must descend from its recorded base")
        );
    }
}

#[test]
fn revision_facts_answer_each_question_from_the_staged_history() {
    let AncestryRepo {
        repo,
        root,
        main,
        side,
        head,
        main_tag,
    } = ancestry_repo();
    let missing = "f".repeat(40);
    let facts = |base: &str, old: &str, main: &str| {
        let facts = request_revision_git_facts(&repo, base, old, &head, Some(main)).unwrap();
        (
            facts.contains_old_head,
            facts.contained_main_oid,
            facts.contained_main_descends_from_base,
        )
    };

    assert_eq!(facts(&main, &main, &main), (true, Some(main.clone()), true));
    assert_eq!(facts(&root, &main, &main), (true, Some(main.clone()), true));
    assert_eq!(
        facts(&side, &side, &main),
        (false, Some(main.clone()), false)
    );
    assert_eq!(facts(&main, &main, &missing), (true, None, false));
    assert_eq!(facts(&main, &main, &main_tag), (true, None, false));
}

fn git_stdout(repo: &FsPath, args: &[&str]) -> String {
    let output = run_git_output(Some(repo), args, "read test git stdout").unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}
