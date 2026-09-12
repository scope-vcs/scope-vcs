use super::*;

#[test]
fn title_only_edit_does_not_read_or_rewrite_the_attachment_journal() {
    let cwd = TempDir::new("title-only-journal");
    let server = MediaFixture::start(false);
    let journal = server
        .server
        .config
        .path()
        .join("scope/request-attachments.json");
    fs::write(&journal, "corrupt journal").unwrap();
    let output = server
        .request_command(cwd.path(), ["edit", "--title", "Renamed"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(server.state.lock().unwrap().edits[0]["title"], "Renamed");
    assert_eq!(fs::read_to_string(journal).unwrap(), "corrupt journal");
}

#[test]
fn cleanup_failures_preserve_saved_results_and_retry_identities() {
    for (operation, mut args) in [
        ("edit", vec!["edit"]),
        ("discussion.start", vec!["discussion", "start"]),
        ("discussion.reply", vec!["discussion", "reply", "dsc_one"]),
        ("discussion.reopen", vec!["discussion", "reopen", "dsc_one"]),
    ] {
        let cwd = TempDir::new("saved-cleanup");
        let attachment = cwd.path().join("photo.png");
        fs::write(&attachment, b"photo").unwrap();
        let server = MediaFixture::start(false);
        let journal = server
            .server
            .config
            .path()
            .join("scope/request-attachments.json");
        let lock = journal.with_extension("lock");
        server.state.lock().unwrap().block_cleanup = Some(lock.clone());
        args.extend(["--attach", attachment.to_str().unwrap()]);
        let output = server.request_command(cwd.path(), &args).output().unwrap();
        assert!(!output.status.success(), "{operation}: {output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        let error: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
        assert_eq!(error["recovery"]["saved"], true, "{operation}: {error}");
        assert_eq!(
            error["recovery"]["operation"],
            format!("request.{operation}")
        );
        assert_eq!(error["recovery"]["request_id"], "req_one");
        let saved_id = if operation == "edit" {
            "request"
        } else {
            "discussion"
        };
        assert!(error["recovery"][saved_id]["id"].is_string());
        if operation.ends_with("reply") || operation.ends_with("reopen") {
            assert_eq!(error["recovery"]["reply"]["id"], "rpl_one");
        }
        let pending: Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
        assert_eq!(pending["uploads"].as_array().unwrap().len(), 1);
        if operation != "edit" {
            assert_eq!(pending["pending_mutations"].as_array().unwrap().len(), 1);
        }
        fs::remove_dir(&lock).unwrap();
        let retry = server.request_command(cwd.path(), &args).output().unwrap();
        assert!(retry.status.success(), "{operation}: {retry:?}");
        let state = server.state.lock().unwrap();
        assert_eq!(
            state.prepare_operation_ids[0],
            state.prepare_operation_ids[1]
        );
        if operation == "discussion.start" {
            assert_eq!(
                state.discussions[0]["client_discussion_id"],
                state.discussions[1]["client_discussion_id"]
            );
        } else if operation != "edit" {
            assert_eq!(
                state.replies[0].1["client_reply_id"],
                state.replies[1].1["client_reply_id"]
            );
        }
    }
}
