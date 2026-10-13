use super::*;

pub(super) fn exercise(contributor: &Actor, maintainer: &Actor, suffix: &str) {
    run_git(&maintainer.repo, ["switch", "main"]);
    maintainer.json(["pull"]);
    let parent_name = format!("e2e-private-parent-{suffix}");
    let child_name = format!("e2e-private-child-{suffix}");
    let parent_path = format!("internal/stack-parent-{suffix}.txt");
    let child_path = format!("internal/stack-child-{suffix}.txt");

    let parent = maintainer.json(["request", "start", &parent_name, "--view", "private"]);
    let parent_id = string_at(&parent, "/result/request/id");
    fs::write(maintainer.repo.join(&parent_path), "parent layer\n").unwrap();
    run_git(&maintainer.repo, ["add", &parent_path]);
    commit_all(&maintainer.repo, "Exercise private parent request");
    let parent_push = maintainer.json(["request", "push"]);
    let parent_head = string_at(&parent_push, "/result/request/head_oid");
    maintainer.json(["request", "submit", "--yes"]);
    let parent_diff = maintainer.json(["request", "diff"]);
    let parent_revision = string_at(&parent_diff, "/result/revisions/comparison/revision_id");

    let child = maintainer.json(["request", "start", &child_name, "--base", &parent_id]);
    let child_id = string_at(&child, "/result/request/id");
    assert_eq!(child["result"]["request"]["base"]["request_id"], parent_id);
    assert_eq!(
        child["result"]["request"]["base"]["revision_id"],
        parent_revision
    );
    assert_eq!(child["result"]["base_oid"], parent_head);
    fs::write(maintainer.repo.join(&child_path), "child layer\n").unwrap();
    run_git(&maintainer.repo, ["add", &child_path]);
    commit_all(&maintainer.repo, "Exercise private child request");
    maintainer.json(["request", "push"]);
    maintainer.json(["request", "submit", "--yes"]);

    let diff = maintainer.json(["request", "diff"]);
    assert_eq!(diff["result"]["revisions"]["comparison"]["mode"], "request");
    let files = diff["result"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "child review included parent work: {diff}");
    assert_eq!(
        files[0]["diff"]["path"]
            .as_str()
            .unwrap()
            .trim_start_matches('/'),
        child_path
    );
    assert_eq!(files[0]["diff"]["new_content"]["text"], "child layer\n");
    let shown = maintainer.json(["request", "show", "--request", &child_id]);
    assert_eq!(shown["result"]["request"]["base"]["head_oid"], parent_head);
    assert_eq!(shown["result"]["request"]["stack_depth"], 2);
    assert_eq!(
        shown["result"]["request"]["stack_status"],
        "waiting_for_parent"
    );
    assert_eq!(
        shown["result"]["request"]["mergeability"]["status"],
        "WaitingForParent"
    );

    for (id, name) in [(&parent_id, &parent_name), (&child_id, &child_name)] {
        assert_error(
            contributor,
            ["request", "show", "--request", id],
            "not_found",
        );
        let public_list = contributor.json(["request", "list", "--search", name]);
        assert_request_absent(&public_list, id);
        let serialized = public_list.to_string();
        assert!(!serialized.contains(name));
        assert!(!serialized.contains(&parent_head));
        assert!(!serialized.contains(&parent_path));
        assert!(!serialized.contains(&child_path));
    }

    maintainer.json(["request", "close", "--request", &parent_id, "--yes"]);
    let blocked = maintainer.json(["request", "show", "--request", &child_id]);
    assert_eq!(blocked["result"]["request"]["state"], "Open");
    assert_eq!(
        blocked["result"]["request"]["stack_status"],
        "parent_closed"
    );
    assert_eq!(
        blocked["result"]["request"]["mergeability"]["status"],
        "ParentClosed"
    );
    let original_head = string_at(&blocked, "/result/request/head_oid");
    let restacked = maintainer.json(["request", "restack", "--onto", "main"]);
    assert_eq!(restacked["result"]["status"], "ready_to_publish");
    assert_eq!(restacked["result"]["request_id"], child_id);
    assert_eq!(restacked["result"]["selected_base"]["kind"], "main");
    assert_eq!(restacked["result"]["original_head_oid"], original_head);
    let backup_ref = string_at(&restacked, "/result/backup_ref");
    assert_eq!(
        git_stdout(&maintainer.repo, ["rev-parse", &backup_ref]),
        original_head
    );
    assert!(!maintainer.repo.join(&parent_path).exists());
    assert_eq!(
        fs::read_to_string(maintainer.repo.join(&child_path)).unwrap(),
        "child layer\n"
    );
    let published = maintainer.json(["request", "push"]);
    assert_eq!(published["result"]["request"]["id"], child_id);
    assert_eq!(published["result"]["request"]["base"]["kind"], "main");
    assert_eq!(published["result"]["request"]["stack_depth"], 1);
    let recovered_diff = maintainer.json(["request", "diff"]);
    let recovered_files = recovered_diff["result"]["files"].as_array().unwrap();
    assert_eq!(
        recovered_files.len(),
        1,
        "reparented review included removed parent work: {recovered_diff}"
    );
    assert_eq!(
        recovered_files[0]["diff"]["path"]
            .as_str()
            .unwrap()
            .trim_start_matches('/'),
        child_path
    );
    assert_eq!(
        recovered_files[0]["diff"]["new_content"]["text"],
        "child layer\n"
    );
    assert_error(
        contributor,
        ["request", "show", "--request", &child_id],
        "not_found",
    );
    maintainer.json(["request", "close", "--request", &child_id, "--yes"]);
    run_git(&maintainer.repo, ["switch", "main"]);
    maintainer.json(["pull"]);
    assert!(!maintainer.repo.join(parent_path).exists());
    assert!(!maintainer.repo.join(child_path).exists());
}
