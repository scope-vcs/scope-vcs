use super::*;

pub(super) fn exercise(contributor: &Actor, maintainer: &Actor, suffix: &str) {
    run_git(&maintainer.repo, ["switch", "main"]);
    maintainer.json(["pull"]);
    let parent_name = format!("e2e-auto-parent-{suffix}");
    let child_name = format!("e2e-auto-child-{suffix}");
    let parent_path = format!("internal/auto-parent-{suffix}.txt");
    let extension_path = format!("internal/auto-extension-{suffix}.txt");
    let child_path = format!("internal/auto-child-{suffix}.txt");

    let parent = maintainer.json(["request", "start", &parent_name, "--view", "private"]);
    let parent_id = string_at(&parent, "/result/request/id");
    fs::write(maintainer.repo.join(&parent_path), "parent layer\n").unwrap();
    run_git(&maintainer.repo, ["add", &parent_path]);
    commit_all(&maintainer.repo, "Private auto-merge parent");
    let parent_push = maintainer.json(["request", "push"]);
    let pinned_parent_head = string_at(&parent_push, "/result/request/head_oid");
    maintainer.json(["request", "submit", "--yes"]);
    let parent_diff = maintainer.json(["request", "diff"]);
    let pinned_parent_revision = string_at(&parent_diff, "/result/revisions/review_revision_id");

    let child = maintainer.json(["request", "start", &child_name, "--base", &parent_id]);
    let child_id = string_at(&child, "/result/request/id");
    fs::write(maintainer.repo.join(&child_path), "child layer\n").unwrap();
    run_git(&maintainer.repo, ["add", &child_path]);
    commit_all(&maintainer.repo, "Private auto-merge child");
    let child_push = maintainer.json(["request", "push"]);
    let child_head = string_at(&child_push, "/result/request/head_oid");
    maintainer.json(["request", "submit", "--yes"]);
    let child_diff = maintainer.json(["request", "diff"]);
    let child_revision = string_at(&child_diff, "/result/revisions/review_revision_id");
    let checks = maintainer.json(["request", "ci", "--request", &child_id]);
    assert!(
        checks["result"]["checks"]["checks"]
            .as_array()
            .unwrap()
            .is_empty(),
        "auto-merge integration requires the seeded repository without required checks: {checks}"
    );
    assert_eq!(
        checks["result"]["checks"]["mergeability"]["status"],
        "WaitingForParent"
    );

    for id in [&parent_id, &child_id] {
        assert_error(
            contributor,
            ["request", "show", "--request", id],
            "not_found",
        );
        assert_error(
            contributor,
            ["request", "merge", "--request", id, "--auto", "--yes"],
            "not_found",
        );
    }
    let authorized_child = maintainer.json([
        "request",
        "merge",
        "--request",
        &child_id,
        "--auto",
        "--yes",
    ]);
    assert_eq!(authorized_child["command"], "request.merge.auto");
    let child_authorization = &authorized_child["result"]["response"];
    assert_eq!(child_authorization["request_id"], child_id);
    assert_eq!(child_authorization["head_oid"], child_head);
    assert_eq!(child_authorization["revision_id"], child_revision);
    assert_eq!(child_authorization["intent"]["status"], "Active");
    assert_eq!(
        child_authorization["waiting_reason"],
        "Waiting for the parent request to land"
    );
    let child_intent = string_at(&authorized_child, "/result/response/intent/id");
    let parent_before = maintainer.json(["request", "show", "--request", &parent_id]);
    assert!(parent_before["result"]["auto_merge"]["intent"].is_null());
    assert_eq!(parent_before["result"]["request"]["state"], "Open");

    run_git(&maintainer.repo, ["switch", &parent_name]);
    fs::write(maintainer.repo.join(&extension_path), "parent extension\n").unwrap();
    run_git(&maintainer.repo, ["add", &extension_path]);
    commit_all(&maintainer.repo, "Extend parent after child authorization");
    let extended = maintainer.json(["request", "push"]);
    let extended_parent_head = string_at(&extended, "/result/request/head_oid");
    let extended_diff = maintainer.json(["request", "diff"]);
    let extended_parent_revision =
        string_at(&extended_diff, "/result/revisions/review_revision_id");
    assert_ne!(extended_parent_head, pinned_parent_head);
    assert_ne!(extended_parent_revision, pinned_parent_revision);
    let retained = maintainer.json(["request", "show", "--request", &child_id]);
    assert_eq!(retained["result"]["request"]["state"], "Open");
    assert_eq!(retained["result"]["request"]["head_oid"], child_head);
    assert_eq!(
        retained["result"]["request"]["base"]["head_oid"],
        pinned_parent_head
    );
    assert_eq!(
        retained["result"]["request"]["base"]["revision_id"],
        pinned_parent_revision
    );
    assert_eq!(
        retained["result"]["auto_merge"]["intent"]["id"],
        child_intent
    );
    assert_eq!(
        retained["result"]["auto_merge"]["intent"]["status"],
        "Active"
    );
    assert_eq!(
        retained["result"]["auto_merge"]["intent"]["head_oid"],
        child_head
    );
    assert_eq!(
        retained["result"]["auto_merge"]["intent"]["revision_id"],
        child_revision
    );
    let parent_checks = maintainer.json(["request", "ci", "--request", &parent_id]);
    assert!(
        parent_checks["result"]["checks"]["checks"]
            .as_array()
            .unwrap()
            .is_empty(),
        "parent extension unexpectedly requires checks: {parent_checks}"
    );
    let parent_unauthed = maintainer.json(["request", "show", "--request", &parent_id]);
    assert!(parent_unauthed["result"]["auto_merge"]["intent"].is_null());

    let authorized_parent = maintainer.json([
        "request",
        "merge",
        "--request",
        &parent_id,
        "--auto",
        "--yes",
    ]);
    let parent_intent = string_at(&authorized_parent, "/result/response/intent/id");
    assert_ne!(parent_intent, child_intent);
    assert_eq!(
        authorized_parent["result"]["response"]["intent"]["head_oid"],
        extended_parent_head
    );
    assert_eq!(
        authorized_parent["result"]["response"]["intent"]["revision_id"],
        extended_parent_revision
    );
    let landed_parent = wait_for_merge(maintainer, &parent_id);
    let landed_child = wait_for_merge(maintainer, &child_id);
    for (landed, intent, head, revision) in [
        (
            &landed_parent,
            &parent_intent,
            &extended_parent_head,
            &extended_parent_revision,
        ),
        (&landed_child, &child_intent, &child_head, &child_revision),
    ] {
        assert_eq!(landed["result"]["request"]["merged_head_oid"], *head);
        assert_eq!(landed["result"]["auto_merge"]["intent"]["id"], *intent);
        assert_eq!(
            landed["result"]["auto_merge"]["intent"]["status"],
            "Fulfilled"
        );
        assert_eq!(landed["result"]["auto_merge"]["intent"]["head_oid"], *head);
        assert_eq!(
            landed["result"]["auto_merge"]["intent"]["revision_id"],
            *revision
        );
    }
    assert_eq!(landed_child["result"]["request"]["base"]["kind"], "main");
    assert_eq!(
        landed_child["result"]["request"]["base"]["review_base_oid"],
        pinned_parent_head
    );
    assert_eq!(
        landed_child["result"]["auto_merge"]["revision_id"],
        child_revision
    );
    assert_eq!(landed_child["result"]["request"]["head_oid"], child_head);
    run_git(&maintainer.repo, ["switch", "main"]);
    maintainer.json(["pull"]);
    for (path, contents) in [
        (&parent_path, "parent layer\n"),
        (&extension_path, "parent extension\n"),
        (&child_path, "child layer\n"),
    ] {
        assert_eq!(
            fs::read_to_string(maintainer.repo.join(path)).unwrap(),
            contents
        );
    }
    for id in [&parent_id, &child_id] {
        assert_error(
            contributor,
            ["request", "show", "--request", id],
            "not_found",
        );
    }
    run_git(&contributor.repo, ["switch", "main"]);
    contributor.json(["pull"]);
    for path in [&parent_path, &extension_path, &child_path] {
        assert!(!contributor.repo.join(path).exists());
    }
}

fn wait_for_merge(actor: &Actor, request_id: &str) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    loop {
        let shown = actor.json(["request", "show", "--request", request_id]);
        if shown["result"]["request"]["state"] == "Merged" {
            return shown;
        }
        assert!(
            matches!(
                shown["result"]["auto_merge"]["intent"]["status"].as_str(),
                Some("Active" | "Fulfilled")
            ),
            "auto-merge stopped before landing: {shown}"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "auto-merge did not land request {request_id}: {shown}"
        );
        thread::sleep(Duration::from_millis(250));
    }
}
