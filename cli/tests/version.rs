use std::process::Command;

#[test]
fn version_reports_package_build_and_protocol_identity() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let build_sha = option_env!("SCOPE_BUILD_SHA").unwrap_or("development");
    assert_eq!(
        stdout,
        format!(
            "scope {} (build {}; protocol {})\n",
            env!("CARGO_PKG_VERSION"),
            build_sha,
            scope_api_contract::CLI_PROTOCOL_VERSION,
        )
    );
}
