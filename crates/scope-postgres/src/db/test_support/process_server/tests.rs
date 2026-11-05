use super::*;
use std::io::{BufRead as _, Read as _, Write as _};

#[test]
fn next_process_reaps_a_killed_owners_server_and_preserves_live_servers() {
    const CHILD: &str = "SCOPE_TEST_SERVER_CLEANUP_CHILD";
    const TEST: &str = "db::test_support::process_server::tests::next_process_reaps_a_killed_owners_server_and_preserves_live_servers";
    if let Ok(mode) = std::env::var(CHILD) {
        let server = ProcessServer::start().unwrap();
        if mode == "orphan" {
            println!("\nSCOPE_TEST_SERVER_ROOT={}", server.root.display());
            std::io::stdout().flush().unwrap();
            std::io::stdin().read_exact(&mut [0]).unwrap();
            unreachable!();
        }
        let orphan = PathBuf::from(std::env::var_os("SCOPE_TEST_ORPHAN_SERVER_ROOT").unwrap());
        assert!(!orphan.exists(), "next process left {}", orphan.display());
        let live = PathBuf::from(std::env::var_os("SCOPE_TEST_LIVE_SERVER_ROOT").unwrap());
        run(Command::new(server.bin.join("pg_ctl"))
            .arg("-D")
            .arg(live.join("data"))
            .arg("status"))
        .unwrap();
        return;
    }
    let live = ProcessServer::start().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD, "orphan")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let root = std::io::BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(Result::unwrap)
        .find_map(|line| {
            line.strip_prefix("SCOPE_TEST_SERVER_ROOT=")
                .map(PathBuf::from)
        })
        .unwrap();
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    let orphan = ProcessServer {
        root,
        bin: live.bin.clone(),
        database_url: String::new(),
    };
    run(Command::new(orphan.bin.join("pg_ctl"))
        .arg("-D")
        .arg(orphan.root.join("data"))
        .arg("status"))
    .unwrap();
    let mut connection =
        std::os::unix::net::UnixStream::connect(orphan.root.join("socket/.s.PGSQL.5432")).unwrap();
    connection
        .set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    let reaper = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD, "reap")
        .env("SCOPE_TEST_ORPHAN_SERVER_ROOT", &orphan.root)
        .env("SCOPE_TEST_LIVE_SERVER_ROOT", &live.root)
        .output()
        .unwrap();
    assert!(
        reaper.status.success(),
        "reaper failed:\n{}\n{}",
        String::from_utf8_lossy(&reaper.stdout),
        String::from_utf8_lossy(&reaper.stderr)
    );
    assert!(!orphan.root.exists());
    assert_eq!(
        connection.read(&mut [0]).unwrap(),
        0,
        "orphan server still holds its client socket"
    );
}
