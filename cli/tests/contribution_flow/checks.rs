use super::*;

// The seeded stack has no job runner. Exercise the real approval/merge gates and
// revision transitions without manufacturing successful check results.
pub(super) fn exercise(contributor: &Actor, maintainer: &Actor, suffix: &str) {
    // Workflow policy belongs to maintainers. Public requests may change code,
    // but cannot introduce their own maintainer-controlled check configuration.
    fs::create_dir_all(maintainer.repo.join(".scope/runs")).unwrap();
    fs::write(
        maintainer.repo.join(".scope/runs/request.yml"),
        r#"name: Request gate
on:
  request: true
container:
  image: ghcr.io/scope/dev-seed-ci@sha256:0000000000000000000000000000000000000000000000000000000000000000
timeout: 5m
jobs:
  verify:
    steps:
      - name: Verify
        run: 'true'
"#,
    )
    .unwrap();
    run_git(&maintainer.repo, ["add", ".scope/runs/request.yml"]);
    commit_all(&maintainer.repo, "Require checks on contributed revisions");
    maintainer.json(["push", "--main", "--no-review", "--wait"]);
    run_git(&contributor.repo, ["switch", "main"]);
    contributor.json(["pull"]);
    assert!(
        !contributor.repo.join(".scope/runs/request.yml").exists(),
        "public clone exposed maintainer workflow definitions"
    );
    let name = format!("e2e-checks-{suffix}");
    let started = contributor.json(["request", "start", &name]);
    let id = string_at(&started, "/result/request/id");
    fs::write(
        contributor.repo.join("check-revision.txt"),
        "first revision\n",
    )
    .unwrap();
    run_git(&contributor.repo, ["add", "check-revision.txt"]);
    commit_all(&contributor.repo, "Exercise request check approval");
    let pushed = contributor.json(["request", "push"]);
    let first_head = string_at(&pushed, "/result/request/head_oid");
    contributor.json(["request", "submit", "--yes"]);

    let waiting = maintainer.json(["request", "checks", "--request", &id]);
    assert_eq!(waiting["result"]["checks"]["state"], "awaiting-approval");
    assert_eq!(waiting["result"]["checks"]["can_approve"], true);
    let public = contributor.json(["request", "checks"]);
    assert_eq!(public["result"]["checks"]["can_approve"], false);
    assert_error(contributor, ["request", "checks", "--approve"], "forbidden");
    assert_error(
        maintainer,
        ["request", "merge", "--request", &id, "--yes"],
        "conflict",
    );

    maintainer.json(["request", "checks", "--request", &id, "--approve"]);
    let queued = maintainer.json(["request", "checks", "--request", &id]);
    assert_eq!(queued["result"]["checks"]["state"], "started");
    assert_eq!(queued["result"]["checks"]["head_oid"], first_head);
    assert_eq!(
        queued["result"]["checks"]["checks"][0]["run_state"],
        "queued"
    );
    assert_eq!(
        queued["result"]["checks"]["mergeability"]["status"],
        "ChecksPending"
    );
    assert_error(
        maintainer,
        ["request", "merge", "--request", &id, "--yes"],
        "conflict",
    );
    assert_error(
        contributor,
        ["request", "merge", "--auto", "--yes"],
        "forbidden",
    );

    let authorized = maintainer.json(["request", "merge", "--request", &id, "--auto", "--yes"]);
    assert_eq!(
        authorized["result"]["response"]["intent"]["status"],
        "Active"
    );
    let canceled = maintainer.json([
        "request",
        "merge",
        "--request",
        &id,
        "--cancel-auto",
        "--yes",
    ]);
    assert_eq!(
        canceled["result"]["response"]["intent"]["status"],
        "Cancelled"
    );
    maintainer.json(["request", "merge", "--request", &id, "--auto", "--yes"]);

    fs::write(
        contributor.repo.join("check-revision.txt"),
        "new revision\n",
    )
    .unwrap();
    run_git(&contributor.repo, ["add", "check-revision.txt"]);
    commit_all(
        &contributor.repo,
        "Invalidate prior check approval and auto merge",
    );
    let pushed = contributor.json(["request", "push"]);
    let second_head = string_at(&pushed, "/result/request/head_oid");
    assert_ne!(second_head, first_head);
    let revised = maintainer.json(["request", "checks", "--request", &id]);
    assert_eq!(revised["result"]["checks"]["head_oid"], second_head);
    assert_eq!(revised["result"]["checks"]["state"], "awaiting-approval");
    assert!(revised["result"]["checks"]["checks"][0]["run_id"].is_null());
    let shown = maintainer.json(["request", "show", "--request", &id]);
    assert_eq!(shown["result"]["auto_merge"]["intent"]["status"], "Stopped");
    assert_eq!(
        shown["result"]["auto_merge"]["intent"]["reason"],
        "RequestChanged"
    );
    assert_eq!(shown["result"]["request"]["state"], "Open");
    maintainer.json(["request", "close", "--request", &id, "--yes"]);
}
