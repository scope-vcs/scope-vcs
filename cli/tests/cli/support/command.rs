use std::{
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn scope_command(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scope"));
    command.current_dir(cwd);
    command.env(
        "SCOPE_API_URL",
        format!("http://127.0.0.1:9/scope-cli-test-{}", unix_nanos()),
    );
    command
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}
