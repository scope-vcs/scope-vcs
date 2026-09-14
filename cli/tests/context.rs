mod support;
use std::fs;
use support::*;

#[test]
fn malformed_checkout_config_fails_session_commands_before_contacting_a_default_endpoint() {
    let dir = TempDir::new("broken-config-session");
    create_repo_with_head(dir.path());
    fs::write(dir.path().join(".git/config"), "[broken").unwrap();
    for command in ["whoami", "login"] {
        let output = scope_command(dir.path())
            .env_remove("SCOPE_API_URL")
            .env_remove("SCOPE_API_PUBLIC_URL")
            .args(["--json", command])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("bad config"), "{command}: {error}");
        assert!(!error.contains("start browser login"));
    }
}
