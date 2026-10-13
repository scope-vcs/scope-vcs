use super::*;

#[test]
fn request_reads_work_outside_a_checkout_with_explicit_repository() {
    let dir = TempDir::new("request-outside");
    let parent = json!({
        "id":"req_parent","name":"parser-core","title":"Parser core","state":"Open",
        "stack_status":"main","stack_depth":1,"head_oid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "current_revision_id":"rev_parent_current"
    });
    let child = json!({
        "id":"req_child","name":"parser-tests","title":"Parser tests","state":"Open",
        "stack_status":"waiting_for_parent","stack_depth":3,"head_oid":OID,
        "current_revision_id":"rev_child"
    });
    let navigation = json!({
        "parent":{"kind":"available","request":parent},"ancestors":[parent],
        "ancestors_unavailable":false,"children":{"requests":[child],"next_cursor":"child-page-two"}
    });
    let mut detail = request();
    detail["base"] = json!({"kind":"request","request_id":"req_parent","revision_id":"rev_parent_pin","head_oid":OID});
    detail["stack_depth"] = 2.into();
    detail["stack_status"] = "parent_extended".into();
    let server = FixtureServer::with_navigation(detail.clone(), detail, navigation.clone());
    for command in ["show", "status"] {
        let value = success(
            server
                .command(dir.path())
                .args([
                    "--json",
                    "--repo",
                    "owner/repo",
                    "request",
                    command,
                    "--request",
                    "req_one",
                ])
                .output()
                .unwrap(),
        );
        assert_eq!(value["result"]["request"]["id"], "req_one");
        assert_eq!(value["result"]["stack_navigation"], navigation);
        assert_eq!(
            value["result"]["request"]["base"]["revision_id"],
            "rev_parent_pin"
        );
        if command == "show" {
            assert_eq!(value["result"]["auto_merge"]["request_id"], "req_one");
        }
        let output = server
            .command(dir.path())
            .args([
                "--repo",
                "owner/repo",
                "request",
                command,
                "--request",
                "req_one",
            ])
            .output()
            .unwrap();
        assert_success(&output, "show authorized stack navigation");
        let human = String::from_utf8(output.stdout).unwrap();
        for expected in [
            "path: parser-core → fix-one",
            "review pin: parser-core @ rev_parent_pin (aaaaaaa)",
            "parent current: parser-core @ rev_parent_current (bbbbbbb)",
            "children: parser-tests (req_child)",
            "more children: next_cursor child-page-two",
        ] {
            assert!(human.contains(expected), "missing {expected:?} in {human}");
        }
    }

    let mut opaque_detail = request();
    opaque_detail["base"] =
        json!({"kind":"request","request_id":"req_hidden","revision_id":"rev_pin","head_oid":OID});
    let opaque = FixtureServer::with_navigation(
        opaque_detail.clone(),
        opaque_detail,
        json!({
            "parent":{"kind":"unavailable"},"ancestors":[],"ancestors_unavailable":true,
            "children":{"requests":[],"next_cursor":null}
        }),
    );
    let output = opaque
        .command(dir.path())
        .args([
            "--repo",
            "owner/repo",
            "request",
            "show",
            "--request",
            "req_one",
        ])
        .output()
        .unwrap();
    assert_success(&output, "show opaque dependency");
    let human = String::from_utf8(output.stdout).unwrap();
    assert!(
        human.contains("review pin: dependency unavailable @ rev_pin"),
        "{human}"
    );
    assert!(!human.contains("req_hidden"), "{human}");
}
