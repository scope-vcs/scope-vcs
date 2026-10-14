use super::*;
use crate::git::test_repo::pinned_identity_git as git;
use crate::http::request_review::tests::read_policy;
use scope_domain::{
    account::UserAccount,
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::ContentRef,
    repository::Repository,
    requests::RequestBase,
    views::ViewId,
};

#[test]
fn request_comparison_keeps_cumulative_net_changes_separate_from_push_and_commit_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let raw_repo = directory.path();
    git(raw_repo, &["init", "--quiet"], None);
    let blob = git(
        raw_repo,
        &["hash-object", "-w", "--stdin"],
        Some("content\n"),
    );
    let tree = |paths: &[&str]| {
        let input = paths
            .iter()
            .map(|path| format!("100644 blob {blob}\t{path}\n"))
            .collect::<String>();
        git(raw_repo, &["mktree"], Some(&input))
    };
    let empty_tree = tree(&[]);
    let root = git(raw_repo, &["commit-tree", &empty_tree, "-m", "root"], None);
    let parent_tree = tree(&["parent.txt"]);
    let parent = git(
        raw_repo,
        &["commit-tree", &parent_tree, "-p", &root, "-m", "parent"],
        None,
    );
    let first_tree = tree(&["child.txt", "parent.txt", "temporary.txt"]);
    let first = git(
        raw_repo,
        &["commit-tree", &first_tree, "-p", &parent, "-m", "first"],
        None,
    );
    let final_tree = tree(&["child.txt", "parent.txt"]);
    let head = git(
        raw_repo,
        &[
            "commit-tree",
            &final_tree,
            "-p",
            &first,
            "-m",
            "revert temporary",
        ],
        None,
    );
    let revision = RequestRevision {
        id: "child-revision-2".to_string(),
        request_id: "child".to_string(),
        position: 2,
        base_version: 1,
        actor_user_id: Some("owner".to_string()),
        old_head_oid: first.clone(),
        new_head_oid: head.clone(),
        base: RequestBase::Request {
            request_id: "parent".to_string(),
            revision_id: "parent-revision-1".to_string(),
            head_oid: parent.clone(),
        },
        main_ancestor_oid: root.clone(),
        rewrote_history: false,
        git_snapshot: SourceBlob {
            content_ref: ContentRef::blob_sha256("snapshot"),
            sha256: "snapshot".to_string(),
            git_oid: "snapshot".to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 1,
        },
        created_at_unix: 1,
    };
    let owner = UserAccount {
        id: "owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.test".to_string(),
        email_verified: true,
    };
    let repo = Repository::new(&owner, "repo", ViewId::public(), "repoi_test").unwrap();
    let policy = read_policy(&repo);
    let access = repo.access_for_user_id(&owner.id);
    let inspect = |mode, commit, budget, path| {
        inspect_comparison(
            raw_repo,
            &policy,
            &access,
            &revision,
            mode,
            commit,
            ComparisonFileSelection {
                limit: budget,
                path,
            },
        )
        .unwrap()
    };
    let aggregate = inspect(RequestComparisonMode::Request, None, 100, None);
    assert_eq!(
        aggregate.base_oid.as_ref().map(|oid| oid.as_str()),
        Some(parent.as_str())
    );
    assert_eq!(
        aggregate.head_oid.as_ref().map(|oid| oid.as_str()),
        Some(head.as_str())
    );
    assert_eq!(
        aggregate
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["child.txt"]
    );
    assert_eq!(
        aggregate.inspection,
        RequestRevisionInspectionState::Complete
    );
    let push = inspect(RequestComparisonMode::Push, None, 100, None);
    assert_eq!(
        push.files
            .iter()
            .map(|file| (file.path.as_str(), file.kind))
            .collect::<Vec<_>>(),
        [("temporary.txt", scope_api_contract::FileChangeKind::Deleted)]
    );
    let commit = inspect(
        RequestComparisonMode::Commit,
        Some(head.as_str()),
        100,
        None,
    );
    assert_eq!(
        commit.base_oid.as_ref().map(|oid| oid.as_str()),
        Some(first.as_str())
    );
    assert_eq!(commit.files[0].path, "temporary.txt");
    let truncated = inspect(RequestComparisonMode::Request, None, 0, None);
    assert!(truncated.files.is_empty());
    assert_eq!(
        truncated.inspection,
        RequestRevisionInspectionState::Incomplete
    );
    let detail = inspect(RequestComparisonMode::Request, None, 1, Some("child.txt"));
    assert_eq!(detail.files[0].path, "child.txt");
    assert_eq!(detail.inspection, RequestRevisionInspectionState::Complete);
    assert!(!commit_belongs_to_revision(raw_repo, &revision, &parent).unwrap());
    assert!(!commit_belongs_to_revision(raw_repo, &revision, &first).unwrap());
    assert!(commit_belongs_to_revision(raw_repo, &revision, &head).unwrap());
    let merged_upstream = RequestRevision {
        old_head_oid: root.clone(),
        ..revision.clone()
    };
    assert!(!commit_belongs_to_revision(raw_repo, &merged_upstream, &parent).unwrap());
    let push_history = request_revision_commits(
        raw_repo,
        &policy,
        access.clone(),
        &merged_upstream,
        None,
        100,
        100,
    )
    .unwrap();
    assert_eq!(
        push_history
            .visible
            .iter()
            .map(|commit| commit.oid.as_str())
            .collect::<Vec<_>>(),
        [first.as_str(), head.as_str()]
    );
    let main_tree = tree(&["upstream.txt"]);
    let main = git(
        raw_repo,
        &[
            "commit-tree",
            &main_tree,
            "-p",
            &root,
            "-m",
            "advanced main",
        ],
        None,
    );
    let main_based_revision = RequestRevision {
        base: RequestBase::Main {
            review_base_oid: main,
        },
        ..revision.clone()
    };
    let merge_base_comparison = inspect_comparison(
        raw_repo,
        &policy,
        &access,
        &main_based_revision,
        RequestComparisonMode::Request,
        None,
        ComparisonFileSelection {
            limit: 100,
            path: None,
        },
    )
    .unwrap();
    assert_eq!(
        merge_base_comparison
            .base_oid
            .as_ref()
            .map(|oid| oid.as_str()),
        Some(root.as_str())
    );
    assert_eq!(
        merge_base_comparison
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["child.txt", "parent.txt"]
    );
}
