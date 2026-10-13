use super::*;

pub(super) fn exercise(contributor: &Actor, maintainer: &Actor, suffix: &str) {
    for extended in [false, true] {
        let scenario = if extended { "extended" } else { "exact" };
        run_git(&contributor.repo, ["switch", "main"]);
        contributor.json(["pull"]);
        let parent_name = format!("e2e-public-{scenario}-parent-{suffix}");
        let child_name = format!("e2e-public-{scenario}-child-{suffix}");
        let parent_path = format!("public-{scenario}-parent-{suffix}.txt");
        let child_path = format!("public-{scenario}-child-{suffix}.txt");

        let parent = contributor.json(["request", "start", &parent_name, "--view", "public"]);
        assert_command(&parent, "request.start");
        assert_eq!(parent["result"]["request"]["view"], "public");
        let parent_id = string_at(&parent, "/result/request/id");
        let parent_head = publish(contributor, &parent_path, "parent layer\n");
        contributor.json(["request", "submit", "--yes"]);
        let parent_diff = contributor.json(["request", "diff"]);
        let parent_revision = string_at(&parent_diff, "/result/revisions/comparison/revision_id");

        let child = contributor.json(["request", "start", &child_name, "--base", &parent_id]);
        assert_command(&child, "request.start");
        let child_id = string_at(&child, "/result/request/id");
        assert_eq!(child["result"]["request"]["view"], "public");
        assert_eq!(child["result"]["request"]["base"]["request_id"], parent_id);
        assert_eq!(
            child["result"]["request"]["base"]["revision_id"],
            parent_revision
        );
        assert_eq!(child["result"]["base_oid"], parent_head);
        let child_head = publish(contributor, &child_path, "child layer\n");
        contributor.json(["request", "submit", "--yes"]);
        let original_diff = contributor.json(["request", "diff"]);
        assert_layer(&original_diff, &parent_head, &child_path);
        let child_revision = string_at(&original_diff, "/result/revisions/comparison/revision_id");
        let checks = maintainer.json(["request", "ci", "--request", &child_id]);
        assert_eq!(checks["result"]["checks"]["state"], "no-checks");
        assert_eq!(
            checks["result"]["checks"]["mergeability"]["status"],
            "WaitingForParent"
        );
        assert_error(
            maintainer,
            ["request", "merge", "--request", &child_id, "--yes"],
            "conflict",
        );

        let landed_parent_head = if extended {
            run_git(&contributor.repo, ["switch", &parent_name]);
            let head = publish(contributor, &parent_path, "parent extension\n");
            run_git(&contributor.repo, ["switch", &child_name]);
            let shown = contributor.json(["request", "show"]);
            assert_eq!(
                shown["result"]["request"]["stack_status"],
                "parent_extended"
            );
            assert_eq!(shown["result"]["request"]["base"]["head_oid"], parent_head);
            head
        } else {
            parent_head.clone()
        };

        let merged = maintainer.json(["request", "merge", "--request", &parent_id, "--yes"]);
        assert_command(&merged, "request.merge");
        assert_eq!(merged["result"]["response"]["request"]["state"], "Merged");
        run_git(&contributor.repo, ["fetch", "origin", "refs/heads/main"]);
        let view_main = git_stdout(&contributor.repo, ["rev-parse", "FETCH_HEAD"]);
        assert_eq!(view_main, landed_parent_head);
        let shown = contributor.json(["request", "show"]);
        assert_eq!(shown["result"]["request"]["head_oid"], child_head);
        assert_eq!(shown["result"]["request"]["base"]["kind"], "main");
        assert_eq!(
            shown["result"]["request"]["base"]["review_base_oid"],
            parent_head
        );
        let retained = contributor.json(["request", "diff"]);
        assert_eq!(
            retained["result"]["revisions"]["comparison"],
            original_diff["result"]["revisions"]["comparison"]
        );
        assert_layer(&retained, &parent_head, &child_path);

        if extended {
            assert_eq!(
                shown["result"]["request"]["stack_status"],
                "needs_main_update"
            );
            assert_eq!(
                shown["result"]["request"]["mergeability"]["status"],
                "NeedsMainUpdate"
            );
            assert_error(
                maintainer,
                ["request", "merge", "--request", &child_id, "--yes"],
                "conflict",
            );
            let restacked = contributor.json(["request", "restack", "--onto", "main"]);
            assert_command(&restacked, "request.restack");
            assert_eq!(restacked["result"]["status"], "ready_to_publish");
            assert_eq!(restacked["result"]["selected_base"]["kind"], "main");
            assert_eq!(
                restacked["result"]["selected_base"]["review_base_oid"],
                view_main
            );
            assert_eq!(restacked["result"]["original_head_oid"], child_head);
            let backup = string_at(&restacked, "/result/backup_ref");
            assert_eq!(
                git_stdout(&contributor.repo, ["rev-parse", &backup]),
                child_head
            );
            assert_eq!(
                fs::read_to_string(contributor.repo.join(&parent_path)).unwrap(),
                "parent extension\n"
            );
            assert_eq!(
                fs::read_to_string(contributor.repo.join(&child_path)).unwrap(),
                "child layer\n"
            );
            let pending = contributor.json(["request", "show"]);
            assert_eq!(pending["result"]["request"]["head_oid"], child_head);
            assert_eq!(
                pending["result"]["request"]["stack_status"],
                "needs_main_update"
            );
            let published = contributor.json(["request", "push"]);
            assert_command(&published, "request.push");
            assert_eq!(
                published["result"]["request"]["base"]["review_base_oid"],
                view_main
            );
            assert_eq!(
                published["result"]["request"]["main_ancestor_oid"],
                view_main
            );
            assert_eq!(published["result"]["request"]["stack_status"], "main");
            let restacked_head = string_at(&published, "/result/request/head_oid");
            assert_ne!(restacked_head, child_head);
            run_git(
                &contributor.repo,
                ["merge-base", "--is-ancestor", &view_main, &restacked_head],
            );
            let revised = contributor.json(["request", "diff"]);
            assert_layer(&revised, &view_main, &child_path);
            assert_ne!(
                string_at(&revised, "/result/revisions/comparison/revision_id"),
                child_revision
            );
            let previous = contributor.json(["request", "diff", "--revision", &child_revision]);
            assert_eq!(
                previous["result"]["revisions"]["comparison"],
                original_diff["result"]["revisions"]["comparison"]
            );
            assert_layer(&previous, &parent_head, &child_path);
        } else {
            assert_eq!(shown["result"]["request"]["stack_status"], "main");
            assert_eq!(shown["result"]["request"]["main_ancestor_oid"], view_main);
            assert_eq!(
                shown["result"]["request"]["mergeability"]["status"],
                "Ready"
            );
            run_git(
                &contributor.repo,
                ["merge-base", "--is-ancestor", &view_main, &child_head],
            );
        }

        let merged = maintainer.json(["request", "merge", "--request", &child_id, "--yes"]);
        assert_eq!(merged["result"]["response"]["request"]["state"], "Merged");
        run_git(&contributor.repo, ["switch", "main"]);
        contributor.json(["pull"]);
        assert_eq!(
            fs::read_to_string(contributor.repo.join(&parent_path)).unwrap(),
            if extended {
                "parent extension\n"
            } else {
                "parent layer\n"
            }
        );
        assert_eq!(
            fs::read_to_string(contributor.repo.join(&child_path)).unwrap(),
            "child layer\n"
        );
        assert!(!contributor.repo.join("internal/notes.md").exists());
        run_git(&maintainer.repo, ["switch", "main"]);
        maintainer.json(["pull"]);
        assert_eq!(
            fs::read_to_string(maintainer.repo.join(&child_path)).unwrap(),
            "child layer\n"
        );
        assert!(maintainer.repo.join("internal/notes.md").is_file());
    }
}

fn publish(actor: &Actor, path: &str, contents: &str) -> String {
    fs::write(actor.repo.join(path), contents).unwrap();
    run_git(&actor.repo, ["add", path]);
    commit_all(&actor.repo, path);
    let pushed = actor.json(["request", "push"]);
    assert_command(&pushed, "request.push");
    string_at(&pushed, "/result/request/head_oid")
}

fn assert_layer(diff: &Value, base: &str, path: &str) {
    assert_command(diff, "request.diff");
    assert_eq!(diff["result"]["revisions"]["comparison"]["base_oid"], base);
    let files = diff["result"]["files"].as_array().unwrap();
    assert_eq!(
        files.len(),
        1,
        "child review included inherited work: {diff}"
    );
    assert_eq!(
        files[0]["diff"]["path"]
            .as_str()
            .unwrap()
            .trim_start_matches('/'),
        path
    );
    assert_eq!(files[0]["diff"]["new_content"]["text"], "child layer\n");
}
