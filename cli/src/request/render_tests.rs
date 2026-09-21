use super::*;
use serde_json::json;

#[test]
fn list_renders_open_state_and_wait() {
    let request: RequestListItemResponse = serde_json::from_value(json!({
        "id": "req_one", "name": "fix-refs", "title": "Fix refs",
        "author_role": "Public", "audience": "Public", "head_oid": oid('b'),
        "state": "Open", "submitted_at_unix": 10, "updated_at_unix": 20,
        "mergeability": {
            "status": "NotMaintainer",
            "current_main_oid": oid('a'),
            "request_head_oid": oid('b'),
            "reason": "repo maintainer required"
        }
    }))
    .unwrap();

    let rendered = request_list_line(&request, 70);
    assert!(rendered.contains("open"), "{rendered}");
    assert!(rendered.contains("1m"), "{rendered}");
}

#[test]
fn detail_uses_server_capabilities_and_renders_invitees_and_submission() {
    let mut request = summary();
    request.state = RequestState::Open;
    request.submitted_at_unix = Some(10);
    request.permissions.can_edit_identity = true;
    request.invitees = serde_json::from_value(json!([{
        "user": {"id": "scope_usr_devon", "handle": "devon"},
        "invited_by_user_id": "scope_usr_author",
        "created_at_unix": 5
    }]))
    .unwrap();

    let rendered = request_detail_lines(&request).join("\n");

    assert!(rendered.contains("open"), "{rendered}");
    assert!(rendered.contains("submitted"), "{rendered}");
    assert!(rendered.contains("@devon"), "{rendered}");
    assert!(rendered.contains("edit"), "{rendered}");
}

#[test]
fn activity_renders_every_wire_event_in_order_and_escapes_free_text() {
    let identity = json!({"title_sha256": oid('a'), "title_byte_count": 5,
        "description_sha256": oid('b'), "description_byte_count": 10});
    let payloads = [
        json!({"Started": {"identity": identity.clone()}}),
        json!({"Submitted": {"head_oid": oid('a')}}),
        json!({"RevisionPushed": {"old_head_oid": oid('a'), "new_head_oid": oid('b'), "note": "note\n\u{1b}[31m"}}),
        json!({"Merged": {"head_oid": oid('b'), "main_oid": oid('c')}}),
        json!({"Closed": {"head_oid": oid('b')}}),
        json!({"IdentityEdited": {"before": identity.clone(), "after": identity}}),
        json!({"DiscussionResolved": {"discussion_id": "discussion\nresolved"}}),
        json!({"DiscussionReopened": {"discussion_id": "discussion\treopened"}}),
        json!({"AutoMergeEnabled": {"intent_id": "ami_one", "revision_id": "rev_one", "head_oid": oid('b')}}),
        json!({"AutoMergeCancelled": {"intent_id": "ami_one", "revision_id": "rev_one", "head_oid": oid('b')}}),
        json!({"AutoMergeStopped": {"intent_id": "ami_two", "revision_id": "rev_one", "head_oid": oid('b'), "reason": "ChecksFailed"}}),
        json!({"AutoMergeFulfilled": {"intent_id": "ami_three", "revision_id": "rev_one", "head_oid": oid('b'), "main_oid": oid('c')}}),
    ];
    let activity: RequestActivityPageResponse = serde_json::from_value(json!({
        "events": payloads.into_iter().enumerate().rev().map(|(i, payload)| event(i as u64 + 1, payload)).collect::<Vec<_>>(),
        "through_position": 12
    })).unwrap();
    let lines = request_activity_lines(&activity);
    assert_eq!(lines.len(), 12);
    for (line, label) in lines.iter().zip([
        "Started request",
        "Submitted",
        "Revision pushed",
        "Merged",
        "Closed",
        "Edited title or description",
        "Resolved discussion",
        "Reopened discussion",
        "Enabled auto-merge",
        "Canceled auto-merge",
        "Stopped auto-merge",
        "Fulfilled auto-merge",
    ]) {
        assert!(line.starts_with(label), "{line}");
        assert!(!line.chars().any(char::is_control), "{line:?}");
    }
    assert!(lines[2].contains("note  [31m"), "{}", lines[2]);
    let main_oid = oid('c');
    assert!(lines[3].contains(short_oid(&main_oid)));
    assert!(lines[10].contains("checks failed"), "{}", lines[10]);
}

#[test]
fn wait_labels_are_concise_and_saturating() {
    assert_eq!(wait_label(None, 3_600), "-");
    assert_eq!(wait_label(Some(3_590), 3_600), "<1m");
    assert_eq!(wait_label(Some(0), 3_600), "1h");
    assert_eq!(wait_label(Some(4_000), 3_600), "<1m");
}

#[test]
fn checks_awaiting_approval_name_each_workflow_and_how_to_start_them() {
    let checks: RequestChecksResponse = serde_json::from_value(json!({
        "request_id": "req_one", "head_oid": oid('b'), "state": "awaiting-approval",
        "message": null, "can_approve": true,
        "checks": [
            {"workflow_path": "/.scope/runs/checks.yml", "workflow_name": "checks\u{001b}[31m",
                "run_id": null, "run_state": null},
            {"workflow_path": "/.scope/runs/lint.yml", "workflow_name": "lint",
                "run_id": null, "run_state": null}
        ],
        "mergeability": {
            "status": "ChecksAwaitingApproval", "current_main_oid": oid('a'),
            "request_head_oid": oid('b'),
            "reason": "checks are waiting for a maintainer to start them"
        }
    }))
    .unwrap();

    let rendered = request_checks_lines(&checks).join("\n");

    assert!(rendered.contains("head bbbbbbb"), "{rendered}");
    assert!(
        rendered.contains("waiting for maintainer approval"),
        "{rendered}"
    );
    assert!(rendered.contains("checks [31m · not started"), "{rendered}");
    assert!(rendered.contains("lint · not started"), "{rendered}");
    assert!(
        rendered.contains("Mergeability: checks are waiting for a maintainer to start them"),
        "{rendered}"
    );
    assert!(
        rendered.contains("scope request checks --approve"),
        "{rendered}"
    );
    assert!(!rendered.contains('\u{1b}'), "{rendered:?}");
}

#[test]
fn started_checks_report_each_run_state_and_a_missing_run() {
    let checks: RequestChecksResponse = serde_json::from_value(json!({
        "request_id": "req_one", "head_oid": oid('b'), "state": "started",
        "message": null, "can_approve": false,
        "checks": [
            {"workflow_path": "/.scope/runs/checks.yml", "workflow_name": "checks",
                "run_id": "run_a", "run_state": "succeeded"},
            {"workflow_path": "/.scope/runs/lint.yml", "workflow_name": "lint",
                "run_id": "run_b", "run_state": "running"},
            {"workflow_path": "/.scope/runs/docs.yml", "workflow_name": "docs",
                "run_id": "run_c", "run_state": null}
        ],
        "mergeability": {
            "status": "ChecksPending", "current_main_oid": oid('a'),
            "request_head_oid": oid('b'), "reason": "checks have not finished"
        }
    }))
    .unwrap();

    let rendered = request_checks_lines(&checks).join("\n");

    assert!(rendered.contains("Checks: started"), "{rendered}");
    assert!(
        rendered.contains("checks · succeeded (run_a)"),
        "{rendered}"
    );
    assert!(rendered.contains("lint · running (run_b)"), "{rendered}");
    assert!(
        rendered.contains("docs · run is gone (run_c)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Mergeability: checks have not finished"),
        "{rendered}"
    );
    assert!(!rendered.contains("--approve"), "{rendered}");
}

#[test]
fn a_head_without_checks_says_so_and_a_broken_workflow_shows_its_message() {
    let no_checks: RequestChecksResponse = serde_json::from_value(json!({
        "request_id": "req_one", "head_oid": oid('b'), "state": "no-checks",
        "message": null, "can_approve": false, "checks": [],
        "mergeability": {
            "status": "Ready", "current_main_oid": oid('a'),
            "request_head_oid": oid('b'), "reason": null
        }
    }))
    .unwrap();
    let broken: RequestChecksResponse = serde_json::from_value(json!({
        "request_id": "req_one", "head_oid": oid('b'), "state": "configuration-error",
        "message": "checks.yml: unknown key 'runs-on'", "can_approve": false, "checks": [],
        "mergeability": {
            "status": "ChecksConfigurationError", "current_main_oid": oid('a'),
            "request_head_oid": oid('b'),
            "reason": "the request head\u{2019}s workflow configuration is invalid"
        }
    }))
    .unwrap();

    let rendered = request_checks_lines(&no_checks).join("\n");
    assert!(rendered.contains("Checks: none asked for"), "{rendered}");
    assert!(rendered.contains("asks for no checks"), "{rendered}");
    assert!(rendered.contains("Mergeability: ready"), "{rendered}");

    let rendered = request_checks_lines(&broken).join("\n");
    assert!(
        rendered.contains("workflow configuration error"),
        "{rendered}"
    );
    assert!(rendered.contains("unknown key 'runs-on'"), "{rendered}");
    assert!(
        rendered.contains("workflow configuration is invalid"),
        "{rendered}"
    );
}

#[test]
fn a_head_nobody_evaluated_says_its_checks_are_not_worked_out() {
    let unevaluated: RequestChecksResponse = serde_json::from_value(json!({
        "request_id": "req_one", "head_oid": oid('b'), "state": null,
        "message": null, "can_approve": false, "checks": [],
        "mergeability": {
            "status": "ChecksNotEvaluated", "current_main_oid": oid('a'),
            "request_head_oid": oid('b'),
            "reason": "checks have not been worked out for this commit yet"
        }
    }))
    .unwrap();

    let rendered = request_checks_lines(&unevaluated).join("\n");

    assert!(
        rendered.contains("Checks: not worked out for this commit yet"),
        "{rendered}"
    );
    assert!(!rendered.contains("asks for no checks"), "{rendered}");
    assert!(
        rendered.contains("Mergeability: checks have not been worked out"),
        "{rendered}"
    );
}

fn summary() -> RequestSummaryResponse {
    serde_json::from_str(
        r#"{
            "id":"req_one","name":"fix-refs","title":"Fix request refs",
            "description_markdown":"Atomic updates","author_user_id":"scope_usr_author",
            "author_role":"Public","audience":"Public",
            "base_main_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "head_oid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","state":"Draft",
            "activity_version":1,
            "submitted_at_unix":null,"closed_at_unix":null,"closed_by_user_id":null,
            "merged_at_unix":null,"merged_by_user_id":null,
            "merged_head_oid":null,"merged_main_oid":null,"created_at_unix":1,
            "updated_at_unix":2,"invitees":[],
            "permissions":{"can_view_activity":false,"can_open_discussion":false,"can_reply_to_discussion":false,"can_wait_after_reply":false,
                "can_edit_identity":false,"can_pull_branch":false,"can_push_branch":false,
                "can_submit":false,
                "can_manage_invitees":false,"can_leave_request":false,
                "can_close":false,"can_merge":false},
            "mergeability":{"status":"Draft",
                "current_main_oid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "request_head_oid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","reason":null}
        }"#,
    )
    .unwrap()
}

fn event(position: u64, payload: serde_json::Value) -> serde_json::Value {
    json!({
        "id": format!("event_{position}"),
        "position": position,
        "actor": {"id": "scope_usr_actor", "handle": "actor"},
        "kind": payload.as_object().unwrap().keys().next().unwrap(),
        "payload": payload,
        "created_at_unix": position * 10
    })
}

fn oid(character: char) -> String {
    std::iter::repeat_n(character, 40).collect()
}
