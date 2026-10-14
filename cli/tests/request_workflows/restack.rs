use super::stacks::{ContextResponse, stack_server_with_base};
use super::*;

struct Layer {
    checkout: TempDir,
    bare: TempDir,
    server: FixtureServer,
    transport: BareRepoTransport,
    original_head: String,
    old_parent: String,
    new_parent: String,
    main: String,
}

impl Layer {
    fn new(conflict: bool, extension: bool, parent_open: bool) -> Self {
        let checkout = TempDir::new("restack-layer");
        create_repo_with_head(checkout.path());
        run_git(checkout.path(), ["config", "user.name", "Scope Test"]);
        run_git(
            checkout.path(),
            ["config", "user.email", "scope@example.test"],
        );
        let main = git_stdout(checkout.path(), ["rev-parse", "HEAD"]);
        fs::write(checkout.path().join("parent.txt"), "old parent\n").unwrap();
        run_git(checkout.path(), ["add", "."]);
        commit_all(checkout.path(), "Old parent");
        let old_parent = git_stdout(checkout.path(), ["rev-parse", "HEAD"]);
        run_git(checkout.path(), ["switch", "-c", "child"]);
        fs::write(checkout.path().join("child.txt"), "child layer\n").unwrap();
        if conflict {
            fs::write(checkout.path().join("parent.txt"), "child edit\n").unwrap();
        }
        run_git(checkout.path(), ["add", "."]);
        commit_all(checkout.path(), "Child layer");
        let original_head = git_stdout(checkout.path(), ["rev-parse", "HEAD"]);
        run_git(
            checkout.path(),
            [
                "switch",
                "--detach",
                if extension { &old_parent } else { &main },
            ],
        );
        fs::write(checkout.path().join("parent.txt"), "new parent\n").unwrap();
        run_git(checkout.path(), ["add", "."]);
        commit_all(checkout.path(), "New parent");
        let new_parent = git_stdout(checkout.path(), ["rev-parse", "HEAD"]);
        run_git(checkout.path(), ["switch", "child"]);
        let bare = TempDir::new("restack-bare");
        run_git(bare.path(), ["init", "--bare"]);
        run_git(
            checkout.path(),
            [
                "push",
                bare.path().to_str().unwrap(),
                &format!("{main}:refs/heads/main"),
                &format!("{new_parent}:refs/heads/parent"),
                &format!("{new_parent}:refs/heads/other"),
                "child:refs/heads/child",
            ],
        );
        let base = json!({"kind":"request","request_id":"req_parent","revision_id":"rev_old_parent","head_oid":old_parent});
        let (server, _) = stack_server_with_base(
            &new_parent,
            &main,
            bare.path(),
            ContextResponse::None,
            Some(base),
            parent_open,
        );
        let transport = BareRepoTransport::new(&server, checkout.path(), bare.path());
        for (key, value) in [
            ("scopeRequestId", "req_child"),
            ("scopeRequestOwner", "owner"),
            ("scopeRequestRepo", "repo"),
            ("scopeRequestRemote", "scope"),
        ] {
            run_git(
                checkout.path(),
                ["config", &format!("branch.child.{key}"), value],
            );
        }
        run_git(
            checkout.path(),
            ["update-ref", "refs/remotes/scope/main", &main],
        );
        run_git(
            checkout.path(),
            ["branch", "--set-upstream-to=scope/main", "child"],
        );
        Self {
            checkout,
            bare,
            server,
            transport,
            original_head,
            old_parent,
            new_parent,
            main,
        }
    }

    fn command(&self, arguments: &[&str]) -> std::process::Output {
        self.transport
            .command(&self.server, self.checkout.path())
            .args(["--json", "request", "restack"])
            .args(arguments)
            .output()
            .unwrap()
    }

    fn assert_binding(&self) {
        assert_eq!(
            git_stdout(
                self.checkout.path(),
                ["config", "branch.child.scopeRequestId"]
            ),
            "req_child"
        );
        assert_eq!(
            git_stdout(
                self.checkout.path(),
                ["rev-parse", "--abbrev-ref", "@{upstream}"]
            ),
            "scope/main"
        );
        assert_eq!(
            git_stdout(self.bare.path(), ["rev-parse", "refs/heads/child"]),
            self.original_head
        );
    }
}

#[test]
fn restack_replays_only_the_child_layer_and_leaves_publication_explicit() {
    for (extension, onto, parent_open) in [
        (false, None, true),
        (true, None, true),
        (false, Some("main"), false),
        (false, Some("req_other"), true),
    ] {
        let layer = Layer::new(false, extension, parent_open);
        run_git(
            layer.checkout.path(),
            ["config", "rebase.updateRefs", "true"],
        );
        run_git(
            layer.checkout.path(),
            ["branch", "unrelated", &layer.original_head],
        );
        let args = onto.map_or(Vec::new(), |onto| vec!["--onto", onto]);
        let value = success(layer.command(&args));
        assert_eq!(value["command"], "request.restack");
        assert_eq!(value["result"]["status"], "ready_to_publish");
        let expected = if onto == Some("main") {
            &layer.main
        } else {
            &layer.new_parent
        };
        assert_eq!(
            git_stdout(layer.checkout.path(), ["rev-parse", "HEAD^"]),
            *expected
        );
        assert_eq!(
            git_stdout(layer.checkout.path(), ["rev-parse", "unrelated"]),
            layer.original_head
        );
        assert_eq!(
            fs::read_to_string(layer.checkout.path().join("child.txt")).unwrap(),
            "child layer\n"
        );
        if onto == Some("main") {
            assert!(!layer.checkout.path().join("parent.txt").exists());
        } else {
            assert_eq!(
                fs::read_to_string(layer.checkout.path().join("parent.txt")).unwrap(),
                "new parent\n"
            );
        }
        let backup = value["result"]["backup_ref"].as_str().unwrap();
        assert_eq!(
            git_stdout(layer.checkout.path(), ["rev-parse", backup]),
            layer.original_head
        );
        let inspected = success(layer.command(&["--continue"]));
        assert_eq!(inspected["result"]["head_oid"], value["result"]["head_oid"]);
        let aborted = layer.command(&["--abort"]);
        assert_eq!(aborted.status.code(), Some(5), "{aborted:?}");
        assert_eq!(
            git_stdout(layer.checkout.path(), ["rev-parse", "HEAD"]),
            value["result"]["head_oid"].as_str().unwrap()
        );
        layer.assert_binding();
    }
}

#[test]
fn restack_conflict_can_continue_staged_resolutions_or_abort_to_the_backup() {
    for mode in ["continue", "abort", "manual_abort"] {
        let abort = mode != "continue";
        let layer = Layer::new(true, false, true);
        let output = layer.command(&[]);
        assert_eq!(output.status.code(), Some(5), "{output:?}");
        let failure: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(
            failure["recovery"]["original_head_oid"],
            layer.original_head
        );
        assert!(layer.checkout.path().join(".git/rebase-merge").exists());
        if !abort {
            let refused = layer.command(&["--discard-selection"]);
            assert_eq!(refused.status.code(), Some(5), "{refused:?}");
            assert!(
                layer
                    .checkout
                    .path()
                    .join(".git/scope-restack.json")
                    .exists()
            );
            assert!(layer.checkout.path().join(".git/rebase-merge").exists());
            fs::write(
                layer.checkout.path().join("parent.txt"),
                "resolved child edit\n",
            )
            .unwrap();
            run_git(layer.checkout.path(), ["add", "parent.txt"]);
        }
        if mode == "manual_abort" {
            run_git(layer.checkout.path(), ["rebase", "--abort"]);
        }
        let value = success(layer.command(&[if abort { "--abort" } else { "--continue" }]));
        assert_eq!(
            value["result"]["status"],
            if abort { "aborted" } else { "ready_to_publish" }
        );
        if abort {
            assert_eq!(
                git_stdout(layer.checkout.path(), ["rev-parse", "HEAD"]),
                layer.original_head
            );
            assert!(
                !layer
                    .checkout
                    .path()
                    .join(".git/scope-restack.json")
                    .exists()
            );
        } else {
            assert_eq!(
                git_stdout(layer.checkout.path(), ["rev-parse", "HEAD^"]),
                layer.new_parent
            );
            assert_eq!(
                fs::read_to_string(layer.checkout.path().join("parent.txt")).unwrap(),
                "resolved child edit\n"
            );
        }
        let backup = failure["recovery"]["backup_ref"].as_str().unwrap();
        assert_eq!(
            git_stdout(layer.checkout.path(), ["rev-parse", backup]),
            layer.original_head
        );
        layer.assert_binding();
        if mode == "manual_abort" {
            let restarted = layer.command(&["--onto", "main"]);
            assert_eq!(restarted.status.code(), Some(5), "{restarted:?}");
            let failure: Value = serde_json::from_slice(&restarted.stderr).unwrap();
            assert_eq!(failure["recovery"]["failed_step"], "rebase");
            assert!(layer.checkout.path().join(".git/rebase-merge").exists());
            success(layer.command(&["--abort"]));
            layer.assert_binding();
        }
    }
}

#[test]
fn restack_refuses_unsafe_initial_states_without_mutating_the_branch() {
    for mode in ["dirty", "operation", "merge", "worktree", "closed"] {
        let layer = Layer::new(false, false, mode != "closed");
        let linked = TempDir::new("restack-other-worktree");
        match mode {
            "dirty" => {
                fs::write(layer.checkout.path().join("untracked.txt"), "preserve me\n").unwrap()
            }
            "operation" => fs::write(
                layer.checkout.path().join(".git/CHERRY_PICK_HEAD"),
                &layer.old_parent,
            )
            .unwrap(),
            "merge" => {
                run_git(
                    layer.checkout.path(),
                    [
                        "merge",
                        "-s",
                        "ours",
                        "--no-ff",
                        "-m",
                        "Merge another branch",
                        &layer.new_parent,
                    ],
                );
            }
            "worktree" => {
                run_git(
                    layer.checkout.path(),
                    [
                        "worktree",
                        "add",
                        "--force",
                        linked.path().to_str().unwrap(),
                        "child",
                    ],
                );
            }
            _ => {}
        }
        let before = git_stdout(layer.checkout.path(), ["rev-parse", "HEAD"]);
        let output = layer.command(&[]);
        assert_eq!(output.status.code(), Some(2), "{mode}: {output:?}");
        assert_eq!(
            git_stdout(layer.checkout.path(), ["rev-parse", "HEAD"]),
            before
        );
        assert!(
            !layer
                .checkout
                .path()
                .join(".git/scope-restack.json")
                .exists()
        );
        assert!(
            git_stdout(
                layer.checkout.path(),
                ["for-each-ref", "refs/scope/restacks/"]
            )
            .is_empty()
        );
    }
}

#[test]
fn discarding_a_stale_selection_preserves_resolutions_and_allows_a_fresh_main_selection() {
    use std::os::unix::fs::PermissionsExt;
    let layer = Layer::new(true, false, true);
    let conflict = layer.command(&["--onto", "main"]);
    assert_eq!(conflict.status.code(), Some(5), "{conflict:?}");
    fs::write(
        layer.checkout.path().join("parent.txt"),
        "resolved child edit\n",
    )
    .unwrap();
    run_git(layer.checkout.path(), ["add", "parent.txt"]);
    let replayed = success(layer.command(&["--continue"]));
    let preserved_head = git_stdout(layer.checkout.path(), ["rev-parse", "HEAD"]);
    let prior_selection = replayed["result"]["selected_base"]["review_base_oid"]
        .as_str()
        .unwrap()
        .to_string();
    let backup = replayed["result"]["backup_ref"]
        .as_str()
        .unwrap()
        .to_string();
    let advanced_main = git_stdout(
        layer.checkout.path(),
        [
            "commit-tree",
            &format!("{}^{{tree}}", layer.main),
            "-p",
            &layer.main,
            "-m",
            "Advance main after selecting the restack base",
        ],
    );
    run_git(
        layer.checkout.path(),
        [
            "push",
            layer.bare.path().to_str().unwrap(),
            &format!("{advanced_main}:refs/heads/main"),
        ],
    );
    let hook = layer.bare.path().join("hooks/pre-receive");
    fs::write(
        &hook,
        "#!/bin/sh\nprintf 'publication rejected\\n' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
    run_git(
        layer.checkout.path(),
        [
            "update-ref",
            "refs/remotes/scope/child",
            &layer.original_head,
        ],
    );
    let rejected = layer
        .transport
        .command(&layer.server, layer.checkout.path())
        .args(["--json", "request", "push"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(5), "{rejected:?}");
    assert!(
        layer
            .checkout
            .path()
            .join(".git/scope-restack.json")
            .exists()
    );
    run_git(
        layer.checkout.path(),
        ["switch", "-c", "other-recovery-branch"],
    );
    let wrong_branch = layer.command(&["--discard-selection"]);
    assert_eq!(wrong_branch.status.code(), Some(5), "{wrong_branch:?}");
    assert!(
        layer
            .checkout
            .path()
            .join(".git/scope-restack.json")
            .exists()
    );
    run_git(layer.checkout.path(), ["switch", "child"]);
    let receipt_key = "scope.request-req_child.revisionContext";
    run_git(
        layer.checkout.path(),
        ["config", receipt_key, "opaque pending receipt"],
    );
    let discarded = success(layer.command(&["--discard-selection"]));
    assert_eq!(discarded["result"]["status"], "selection_discarded");
    assert_eq!(discarded["result"]["head_oid"], preserved_head);
    assert_eq!(
        discarded["result"]["selected_base"],
        replayed["result"]["selected_base"]
    );
    assert_eq!(discarded["result"]["backup_ref"], backup);
    assert!(
        !layer
            .checkout
            .path()
            .join(".git/scope-restack.json")
            .exists()
    );
    assert_eq!(
        git_stdout(layer.checkout.path(), ["rev-parse", "HEAD"]),
        preserved_head
    );
    assert_eq!(
        git_stdout(layer.checkout.path(), ["rev-parse", &backup]),
        layer.original_head
    );
    assert_eq!(
        git_stdout(
            layer.checkout.path(),
            ["rev-parse", "refs/remotes/scope/child"]
        ),
        layer.original_head
    );
    assert_eq!(
        fs::read_to_string(layer.checkout.path().join("parent.txt")).unwrap(),
        "resolved child edit\n"
    );
    layer.assert_binding();
    assert_eq!(
        git_stdout(layer.checkout.path(), ["config", "--get", receipt_key]),
        "opaque pending receipt"
    );
    run_git(layer.checkout.path(), ["config", "--unset", receipt_key]);
    run_git(
        layer.checkout.path(),
        ["rebase", "--onto", &advanced_main, &prior_selection],
    );
    assert_eq!(
        git_stdout(layer.checkout.path(), ["rev-parse", "HEAD^"]),
        advanced_main
    );
    layer.transport.capture_request_headers();
    let headers = layer.checkout.path().join("fresh-main-headers");
    let refreshed = layer
        .transport
        .command(&layer.server, layer.checkout.path())
        .env("SCOPE_STACK_HEADERS", &headers)
        .args(["--json", "request", "push", "--base", "main"])
        .output()
        .unwrap();
    assert_eq!(refreshed.status.code(), Some(5), "{refreshed:?}");
    assert!(
        fs::read_to_string(headers)
            .unwrap()
            .contains(&format!("Scope-Request-Base: main:{advanced_main}"))
    );
    assert_eq!(
        fs::read_to_string(layer.checkout.path().join("parent.txt")).unwrap(),
        "resolved child edit\n"
    );
    layer.assert_binding();
}
