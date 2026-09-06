mod support;

use scope_cli::repo_config::{repo_config_path, write_worktree_scope_repo_config_with_base};
use scope_domain::repo_config::{ConfigVisibility, RepoConfig, RepoConfigVisibilityRule};
use serde_json::Value;
use std::{fs, path::Path};
use support::*;

fn fixture() -> (TempDir, RepoConfig) {
    let dir = TempDir::new("visibility");
    create_repo_with_head(dir.path());
    fs::create_dir_all(dir.path().join("docs/private")).unwrap();
    fs::create_dir_all(dir.path().join(".scope/runs")).unwrap();
    fs::write(dir.path().join("docs/guide.md"), "guide").unwrap();
    fs::write(dir.path().join("docs/private/key"), "private").unwrap();
    fs::write(dir.path().join(".scope/runs/check.yml"), "workflow").unwrap();
    let mut config = RepoConfig::with_default_visibility(ConfigVisibility::Private);
    config.visibility.rules = vec![
        RepoConfigVisibilityRule {
            path: "/docs/**".to_string(),
            visibility: ConfigVisibility::Public,
        },
        RepoConfigVisibilityRule {
            path: "/docs/private/**".to_string(),
            visibility: ConfigVisibility::Private,
        },
    ];
    write_worktree_scope_repo_config_with_base(dir.path(), &config).unwrap();
    (dir, config)
}

fn json(cwd: &Path, args: &[&str]) -> Value {
    let output = scope_command(cwd)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "visibility JSON command: {output:?}"
    );
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    document["result"].clone()
}

#[test]
fn show_explain_and_validate_agree_with_domain_and_preserve_local_state() {
    let (dir, _) = fixture();
    let config_path = repo_config_path(dir.path()).unwrap();
    let state_path = config_path.with_file_name("repo-state.json");
    let before_config = fs::read(&config_path).unwrap();
    let before_state = fs::read(&state_path).unwrap();
    let show = json(dir.path(), &["visibility", "show"]);
    let paths = show["paths"].as_array().unwrap();
    for (path, visibility, rule) in [
        ("/README.md", "private", "inherited default"),
        ("/docs/guide.md", "public", "inherited /docs/**"),
        ("/docs/private/key", "private", "inherited /docs/private/**"),
        ("/.scope/RULES.md", "public", "forced public"),
        ("/.scope/runs/check.yml", "private", "forced private"),
    ] {
        let shown = paths.iter().find(|entry| entry["path"] == path).unwrap();
        assert_eq!(shown["visibility"], visibility);
        assert_eq!(shown["rule"], rule);
        let explained = json(dir.path(), &["visibility", "explain", path]);
        assert_eq!(explained["path"], *shown);
    }
    let directory = json(dir.path(), &["visibility", "explain", "docs"]);
    assert_eq!(directory["path"]["visibility"], "mixed");
    let validated = json(dir.path(), &["visibility", "validate"]);
    assert_eq!(validated["valid"], true);
    assert_eq!(fs::read(config_path).unwrap(), before_config);
    assert_eq!(fs::read(state_path).unwrap(), before_state);
}

#[test]
fn inspection_and_noninteractive_edit_do_not_initialize_missing_state() {
    let dir = TempDir::new("visibility-missing");
    create_repo_with_head(dir.path());
    for args in [
        vec!["visibility", "show"],
        vec!["visibility", "validate"],
        vec!["visibility", "edit"],
    ] {
        let output = scope_command(dir.path())
            .arg("--json")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        let document: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(document["code"], "bad_request");
        assert!(
            !repo_config_path(dir.path())
                .unwrap()
                .parent()
                .unwrap()
                .exists()
        );
    }
}

#[test]
fn preview_compares_proposed_policy_without_saving_or_changing_managed_paths() {
    let (dir, mut candidate) = fixture();
    candidate.visibility.default = ConfigVisibility::Public;
    let candidate_dir = TempDir::new("visibility-proposal");
    let candidate_path = candidate_dir.path().join("repo.json");
    fs::write(&candidate_path, serde_json::to_vec(&candidate).unwrap()).unwrap();
    let config_path = repo_config_path(dir.path()).unwrap();
    let original = fs::read(&config_path).unwrap();
    let result = json(
        dir.path(),
        &[
            "visibility",
            "preview",
            "--config",
            candidate_path.to_str().unwrap(),
        ],
    );
    assert_eq!(result["config_changed"], true);
    assert!(
        result["comparison"]
            .as_str()
            .unwrap()
            .starts_with("offline:")
    );
    let changes = result["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["path"], "/README.md");
    assert_eq!(changes[0]["before"], "private");
    assert_eq!(changes[0]["after"], "public");
    assert_eq!(fs::read(config_path).unwrap(), original);
}

#[test]
fn validate_reports_invalid_configuration_as_usage_without_rewriting_it() {
    let (dir, _) = fixture();
    let config_path = repo_config_path(dir.path()).unwrap();
    let invalid = b"{\"visibility\":{\"default\":\"public\",\"rules\":[{\"path\":\"../secret\",\"visibility\":\"private\"}]}}";
    fs::write(&config_path, invalid).unwrap();
    let output = scope_command(dir.path())
        .args(["--json", "visibility", "validate"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(fs::read(config_path).unwrap(), invalid);
}

#[cfg(unix)]
#[test]
fn show_and_explain_preserve_filename_identity_and_escape_terminal_control_characters() {
    let (dir, _) = fixture();
    let name = " leading -> filename\t\n";
    fs::write(dir.path().join(name), "private").unwrap();
    let result = json(dir.path(), &["visibility", "explain", name]);
    assert_eq!(result["path"]["path"], format!("/{name}"));
    assert_eq!(result["path"]["visibility"], "private");
    assert_eq!(result["present_in_worktree"], true);
    let output = scope_command(dir.path())
        .args(["visibility", "explain", name])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "visibility text explanation: {output:?}"
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("\\t\\n"), "{text:?}");
    assert_eq!(text.lines().count(), 1);
}
