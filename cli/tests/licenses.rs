mod support;

use std::io::Read;
use std::process::Stdio;

use support::{TempDir, scope_command};

#[test]
fn licenses_exits_quietly_when_the_pipe_reader_closes() {
    let dir = TempDir::new("licenses-closed-pipe");
    for args in [vec!["licenses"], vec!["licenses", "--json"]] {
        let mut child = scope_command(dir.path())
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut reader = child.stdout.take().unwrap();
        reader.read_exact(&mut [0; 16]).unwrap();
        drop(reader);
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
}

#[test]
fn licenses_prints_complete_embedded_texts_without_a_repository_or_server() {
    let dir = TempDir::new("licenses-offline");
    let output = scope_command(dir.path())
        .env("XDG_CONFIG_HOME", dir.path())
        .arg("licenses")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("Scope — Apache-2.0\n"));
    assert!(stdout.contains(include_str!("../../LICENSE")));
    assert!(stdout.contains(include_str!("../../NOTICE")));
    assert!(stdout.contains(include_str!("../../legal/third-party-rust.txt")));
}

#[test]
fn licenses_json_contains_complete_texts_as_one_document() {
    let dir = TempDir::new("licenses-json-offline");
    for args in [["--json", "licenses"], ["licenses", "--json"]] {
        let output = scope_command(dir.path())
            .env("XDG_CONFIG_HOME", dir.path())
            .args(args)
            .output()
            .unwrap();

        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(document["license"], "Apache-2.0");
        assert_eq!(document["license_text"], include_str!("../../LICENSE"));
        assert_eq!(document["notice"], include_str!("../../NOTICE"));
        assert_eq!(
            document["third_party_notices"],
            include_str!("../../legal/third-party-rust.txt")
        );
    }
}
