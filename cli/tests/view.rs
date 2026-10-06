mod support;

use scope_cli::repo_config::{
    load_worktree_scope_repo_config, load_worktree_scope_repo_config_base_hash,
    write_worktree_scope_repo_config_with_base,
};
use scope_domain::{repo_config::RepoConfig, views::ViewId};
use serde_json::Value;
use support::*;

#[test]
fn view_edits_change_only_the_local_config_until_publish() {
    let dir = TempDir::new("view-edits");
    create_repo_with_head(dir.path());
    let published = RepoConfig::with_default_view(ViewId::private());
    write_worktree_scope_repo_config_with_base(dir.path(), &published).unwrap();
    let base = load_worktree_scope_repo_config_base_hash(dir.path()).unwrap();

    let added = scope_command(dir.path())
        .args([
            "view",
            "add",
            "agent",
            "--name",
            "Agent",
            "--include",
            "public",
        ])
        .output()
        .unwrap();
    assert_success(&added, "scope view add");
    assert_eq!(
        String::from_utf8(added.stdout).unwrap(),
        "Added view agent (Agent). Publish it with scope push --main.\n"
    );

    let listed = scope_command(dir.path())
        .args(["--json", "view", "list"])
        .output()
        .unwrap();
    assert_success(&listed, "scope view list");
    let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(listed["result"]["views"][2]["id"], "agent");
    assert_eq!(listed["result"]["views"][2]["includes"][0], "public");

    let refused = scope_command(dir.path())
        .args(["view", "remove", "private"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));

    let local = load_worktree_scope_repo_config(dir.path()).unwrap();
    assert_eq!(
        local.views().display_name(&ViewId::parse("agent").unwrap()),
        "Agent"
    );
    assert_eq!(
        load_worktree_scope_repo_config_base_hash(dir.path()).unwrap(),
        base
    );
}
