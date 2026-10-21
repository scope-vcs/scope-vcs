use super::{ViewId, ensure_request_paths_allowed, new_client_mutation_id, start_view};
use crate::{
    api::RepositoryActor,
    error::CliError,
    git_repo::GitRepo,
    test_support::{TempDir, commit_all},
};
use scope_domain::views::Views;
use std::fs;

#[test]
fn client_mutation_ids_are_unique_across_kinds() {
    let mut ids = std::collections::HashSet::new();
    for kind in ["discussion", "reply", "discussion", "reply"] {
        assert!(ids.insert(new_client_mutation_id(kind).unwrap()));
    }
}

#[test]
fn request_preflight_rejects_protected_add_edit_delete_and_rename_outside_the_full_view() {
    let dir = TempDir::git_repo("request-protected-path-operations", "main");
    fs::write(dir.path().join("README.md"), "base\n").unwrap();
    fs::create_dir_all(dir.path().join(".scope")).unwrap();
    fs::write(dir.path().join(".scope/edited.md"), "before\n").unwrap();
    fs::write(dir.path().join(".scope/deleted.md"), "delete me\n").unwrap();
    fs::write(dir.path().join(".scope/renamed.md"), "rename me\n").unwrap();
    let base_oid = commit_all(dir.path(), "base");

    fs::write(dir.path().join(".scope/added.md"), "added\n").unwrap();
    fs::write(dir.path().join(".scope/edited.md"), "after\n").unwrap();
    fs::remove_file(dir.path().join(".scope/deleted.md")).unwrap();
    fs::create_dir_all(dir.path().join("docs")).unwrap();
    dir.run_git(["mv", ".scope/renamed.md", "docs/renamed.md"]);
    let head_oid = commit_all(dir.path(), "change protected paths");
    let repo = GitRepo {
        root: dir.path().to_path_buf(),
    };
    let views = agent_views();

    for (view, name) in [("public", "Public"), ("agent", "Agent")] {
        let request = request_summary(view, &base_oid, &head_oid);
        let error = ensure_request_paths_allowed(&repo, &views, &request, &base_oid, &head_oid)
            .unwrap_err();
        let structured = error.downcast_ref::<CliError>().unwrap();

        assert_eq!(
            structured.response().code,
            scope_api_contract::ErrorCode::ProtectedPath
        );
        assert_eq!(
            structured.response().fields.paths,
            [
                ".scope/added.md",
                ".scope/deleted.md",
                ".scope/edited.md",
                ".scope/renamed.md",
            ]
        );
        assert!(
            error.to_string().starts_with(&format!(
                "{name} requests cannot change maintainer-controlled paths: "
            )),
            "{error}"
        );
        assert!(error.to_string().contains(".scope/renamed.md"));
        assert_eq!(crate::error::exit_code(&error), 4);
    }
    let private = request_summary("private", &base_oid, &head_oid);
    ensure_request_paths_allowed(&repo, &views, &private, &base_oid, &head_oid).unwrap();
}

#[test]
fn public_request_preflight_excludes_main_only_protected_paths_after_true_divergence() {
    let dir = TempDir::git_repo("request-current-public-main", "main");
    fs::write(dir.path().join("README.md"), "base\n").unwrap();
    let original_base_oid = commit_all(dir.path(), "base");
    dir.run_git(["branch", "request"]);

    fs::create_dir_all(dir.path().join(".scope")).unwrap();
    fs::write(dir.path().join(".scope/RULES.md"), "maintainer change\n").unwrap();
    let current_main_oid = commit_all(dir.path(), "advance public main");

    dir.run_git(["checkout", "request"]);
    fs::write(dir.path().join("README.md"), "request change\n").unwrap();
    let head_oid = commit_all(dir.path(), "request change");
    let request = request_summary("public", &original_base_oid, &head_oid);
    let repo = GitRepo {
        root: dir.path().to_path_buf(),
    };

    ensure_request_paths_allowed(
        &repo,
        &agent_views(),
        &request,
        &current_main_oid,
        &head_oid,
    )
    .unwrap();
}

#[test]
fn a_new_request_takes_the_current_branch_only_when_it_holds_scope_main() {
    let dir = TempDir::git_repo("request-current-branch", "main");
    dir.run_git(["config", "user.email", "scope@example.test"]);
    dir.run_git(["config", "user.name", "Scope Test"]);
    dir.run_git(["commit", "--allow-empty", "-m", "base"]);
    let old_main = git_oid(&dir);
    dir.run_git(["switch", "-c", "docs/rules"]);
    dir.run_git(["commit", "--allow-empty", "-m", "work"]);
    dir.run_git(["switch", "main"]);
    dir.run_git(["commit", "--allow-empty", "-m", "main moved"]);
    let new_main = git_oid(&dir);
    let repo = GitRepo {
        root: dir.path().to_path_buf(),
    };
    let refusal = |base: &str| {
        let error = super::local::adoptable_current_branch(&repo, base).unwrap_err();
        assert_eq!(crate::error::exit_code(&error), 2);
        error.to_string()
    };

    assert!(refusal(&new_main).contains("main cannot become a request branch"));
    dir.run_git(["switch", "docs/rules"]);
    assert!(refusal(&new_main).contains("does not contain the selected base"));
    assert_eq!(
        super::local::adoptable_current_branch(&repo, &old_main).unwrap(),
        "docs/rules"
    );
    dir.run_git(["config", "branch.docs/rules.scopeRequestId", "req_taken"]);
    assert!(refusal(&old_main).contains("already belongs to request req_taken"));
}

#[test]
fn tracking_a_request_keeps_an_upstream_on_another_remote() {
    let dir = TempDir::git_repo("request-keeps-upstream", "main");
    dir.run_git(["config", "user.email", "scope@example.test"]);
    dir.run_git(["config", "user.name", "Scope Test"]);
    dir.run_git(["commit", "--allow-empty", "-m", "base"]);
    let head = git_oid(&dir);
    dir.run_git(["config", "branch.main.remote", "origin"]);
    dir.run_git(["config", "branch.main.merge", "refs/heads/docs/rules"]);
    let repo = GitRepo {
        root: dir.path().to_path_buf(),
    };
    let target = crate::git_transport::ScopeRemote::parse(
        "https://scope.example",
        "scope",
        "https://scope.example/git/private/owner/repo",
    )
    .unwrap();
    let config = |key: &str| {
        String::from_utf8(dir.run_git(["config", "--get", key]).stdout)
            .unwrap()
            .trim()
            .to_string()
    };

    super::local::track_request_branch_ref(&repo, "main", &target, "rules", &head).unwrap();
    assert_eq!(config("branch.main.remote"), "origin");
    assert_eq!(config("branch.main.merge"), "refs/heads/docs/rules");
    dir.run_git(["rev-parse", "--verify", "refs/remotes/scope/rules"]);

    dir.run_git(["config", "--unset", "branch.main.remote"]);
    super::local::track_request_branch_ref(&repo, "main", &target, "rules", &head).unwrap();
    assert_eq!(config("branch.main.remote"), "scope");
    assert_eq!(config("branch.main.merge"), "refs/heads/rules");
}

fn git_oid(dir: &TempDir) -> String {
    String::from_utf8(dir.run_git(["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_string()
}

fn agent_views() -> Views {
    serde_json::from_value(serde_json::json!([
        {"id": "public", "name": "Public", "includes": [], "readers": "anyone"},
        {"id": "private", "name": "Private", "includes": "all", "readers": "assigned"},
        {"id": "agent", "name": "Agent", "includes": ["public"], "readers": "assigned"},
    ]))
    .unwrap()
}

fn request_summary(
    view: &str,
    base_oid: &str,
    head_oid: &str,
) -> crate::api::RequestSummaryResponse {
    let detail: crate::api::RequestDetailResponse = serde_json::from_value(serde_json::json!({
        "stack_navigation":{"parent":null,"ancestors":[],"ancestors_unavailable":false,"children":{"requests":[],"next_cursor":null}},
        "request": {
            "id":"req_one","name":"change","title":"Change",
            "description_markdown":"","author_user_id":"scope_usr_author",
            "author_role":"Member","view":view,
            "base":{"kind":"main","review_base_oid":base_oid},"base_version":1,"stack_status":"main","stack_depth":1,"main_ancestor_oid":base_oid,"head_oid":head_oid,"state":"Draft",
            "activity_version":1,"submitted_at_unix":null,"closed_at_unix":null,
            "closed_by_user_id":null,"merged_at_unix":null,"merged_by_user_id":null,
            "merged_head_oid":null,"merged_main_oid":null,"created_at_unix":1,
            "updated_at_unix":2,"invitees":[],
            "permissions":{"can_view_activity":false,"can_open_discussion":false,
                "can_reply_to_discussion":false,"can_wait_after_reply":false,"can_edit_identity":false,
                "can_pull_branch":false,"can_push_branch":true,"can_submit":false,
                "can_manage_invitees":false,"can_leave_request":false,
                "can_close":false,"can_merge":false},
            "mergeability":{"status":"Draft","current_main_oid":base_oid,
                "request_head_oid":head_oid,"reason":null}
        }
    }))
    .unwrap();
    detail.request
}

fn access(actor: RepositoryActor, view: &str) -> crate::api::RepositoryAccessResponse {
    crate::api::RepositoryAccessResponse {
        actor,
        view: ViewId::parse(view).unwrap(),
        can_push: false,
        can_change_file_visibility: false,
        can_manage_members: false,
        can_delete_repo: false,
    }
}

fn requested(view: &str) -> Option<ViewId> {
    Some(ViewId::parse(view).unwrap())
}

#[test]
fn request_view_defaults_follow_repository_access() {
    let views = agent_views();
    for (actor, view) in [
        (RepositoryActor::Owner, "private"),
        (RepositoryActor::Member, "private"),
        (RepositoryActor::Member, "agent"),
        (RepositoryActor::Public, "public"),
    ] {
        assert_eq!(
            start_view(&access(actor, view), &views, None).unwrap(),
            ViewId::parse(view).unwrap()
        );
    }
}

#[test]
fn requests_start_in_any_view_their_author_can_read() {
    let views = agent_views();
    let agent_member = access(RepositoryActor::Member, "agent");
    let owner = access(RepositoryActor::Owner, "private");
    for (author, view) in [
        (&agent_member, "public"),
        (&owner, "agent"),
        (&owner, "public"),
    ] {
        assert_eq!(
            start_view(author, &views, requested(view)).unwrap(),
            ViewId::parse(view).unwrap()
        );
    }

    let refusal = |author, view| {
        let error = start_view(author, &views, requested(view)).unwrap_err();
        assert_eq!(crate::error::exit_code(&error), 2);
        error.to_string()
    };
    assert!(
        refusal(&agent_member, "private")
            .contains("requests in the Private view need an author who can read it")
    );
    assert!(refusal(&owner, "ops").contains("this repository has no ops view"));
    assert!(
        refusal(&access(RepositoryActor::Public, "public"), "agent")
            .contains("public contributors can only create requests in the Public view")
    );
}

#[test]
fn discussion_start_anchors_whole_request_line_ranges() {
    use super::{RequestDiscussionStartArgs, discussion_start_anchor};
    use clap::Parser;
    let parse = |args: &[&str]| {
        RequestDiscussionStartArgs::try_parse_from(
            ["start", "--body", "Looks off"].iter().chain(args),
        )
    };

    let anchor = discussion_start_anchor(
        &parse(&["--revision", "rev_1", "--path", "src/lib.rs", "--line", "9"]).unwrap(),
    )
    .unwrap();
    assert_eq!(anchor.commit_oid, None);
    assert_eq!(anchor.path.as_deref(), Some("src/lib.rs"));
    assert_eq!(
        anchor.lines,
        Some(crate::api::RequestDiscussionLineRange {
            side: scope_api_contract::RequestDiffSide::New,
            start_line: 9,
            end_line: 9,
        })
    );

    let range = discussion_start_anchor(
        &parse(&[
            "--revision",
            "rev_1",
            "--path",
            "src/lib.rs",
            "--side",
            "old",
            "--start-line",
            "7",
            "--line",
            "9",
        ])
        .unwrap(),
    )
    .unwrap()
    .lines
    .unwrap();
    assert_eq!(
        (range.side, range.start_line, range.end_line),
        (scope_api_contract::RequestDiffSide::Old, 7, 9)
    );

    for invalid in [
        &["--revision", "rev_1", "--line", "9"][..],
        &["--revision", "rev_1", "--path", "a", "--line", "0"],
        &["--revision", "rev_1", "--path", "a", "--start-line", "2"],
    ] {
        assert!(parse(invalid).is_err(), "{invalid:?}");
    }
}
