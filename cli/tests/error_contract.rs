mod support;

use support::*;

#[test]
fn conflicting_login_arguments_use_the_usage_exit_category() {
    let dir = TempDir::new("local-error-contract");
    let config = TempDir::new("local-error-config");

    let invalid_login = scope_command(dir.path())
        .env("XDG_CONFIG_HOME", config.path())
        .args(["login", "--headless", "--exchange", "fixture"])
        .output()
        .unwrap();
    assert_eq!(invalid_login.status.code(), Some(2), "{invalid_login:?}");
    assert!(
        String::from_utf8(invalid_login.stderr)
            .unwrap()
            .contains("cannot be used with")
    );
}
