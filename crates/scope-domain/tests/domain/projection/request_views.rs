use super::*;
use scope_domain::{
    reviewed_updates::content::apply_request_merge_to_repo,
    views::{ViewDefinition, ViewIncludes, ViewReaders},
};

const BASE: &str = "0000000000000000000000000000000000000000";
const FIRST: &str = "1111111111111111111111111111111111111111";
const HEAD: &str = "2222222222222222222222222222222222222222";
const MERGED_MAIN: &str = "3333333333333333333333333333333333333333";

fn agent() -> ViewId {
    ViewId::parse("agent").unwrap()
}

fn agent_config() -> RepoConfig {
    let mut definitions = Vec::<ViewDefinition>::from(Views::builtin());
    definitions.push(ViewDefinition {
        id: agent(),
        name: "Agent".into(),
        includes: ViewIncludes::Some([ViewId::public()].into()),
        readers: ViewReaders::Assigned,
    });
    let mut config = config(ViewId::public(), None, None);
    config.views = Views::new(definitions).unwrap();
    config.files.rules = vec![
        RepoConfigFileRule {
            path: "/src/**".into(),
            view: agent(),
        },
        RepoConfigFileRule {
            path: "/ops/**".into(),
            view: ViewId::private(),
        },
    ];
    config.validate().unwrap();
    config
}

fn agent_repo() -> Repository {
    let mut repo = published_repo_with_public_file("initial", "/README.md", "hello");
    repo.repo_config = agent_config();
    repo
}

fn native(oid: &str, parent: &str, paths: &[&str]) -> NativeRequestCommit {
    NativeRequestCommit {
        oid: oid.into(),
        parent_oids: vec![parent.into()],
        tree_oid: format!("tree-{oid}"),
        changed_paths: paths.iter().map(|value| path(value)).collect(),
    }
}

fn agent_origin(commits: Vec<NativeRequestCommit>) -> RequestMergeOrigin {
    RequestMergeOrigin::View {
        request_id: "request-agent".into(),
        view: agent(),
        base_oid: BASE.into(),
        parent_oids: vec![BASE.into()],
        request_head_oid: commits.last().unwrap().oid.clone(),
        commits,
    }
}

fn merge(
    repo: &mut Repository,
    changes: Vec<ReviewedContentChange>,
    origin: RequestMergeOrigin,
) -> Result<(), ReviewedUpdateError> {
    let config = repo.repo_config.clone();
    apply_request_merge_to_repo(
        repo,
        reviewed_update(
            MERGED_MAIN,
            "Merge agent request",
            changes,
            Some(config.clone()),
            config,
        ),
        origin,
    )
}

#[test]
fn an_agent_request_merge_is_preserved_only_in_the_agent_projection() {
    let mut repo = agent_repo();
    merge(
        &mut repo,
        vec![
            reviewed_change("/README.md", Some("agent edit")),
            reviewed_change("/src/lib.rs", Some("agent code")),
        ],
        agent_origin(vec![
            native(FIRST, BASE, &["/src/lib.rs"]),
            native(HEAD, FIRST, &["/README.md"]),
        ]),
    )
    .unwrap();
    let views = repo.repo_config.views().clone();
    let project =
        |view: &ViewId| project_graph(&repo.graph, &repo.visibility_change_sets, &views, view);

    let agent_projection = project(&agent());
    assert_eq!(
        agent_projection.commits[1..]
            .iter()
            .map(|commit| &commit.materialization)
            .collect::<Vec<_>>(),
        [
            &ProjectionMaterialization::PreserveGitCommit {
                oid: FIRST.into(),
                parent_oids: vec![BASE.into()],
                tree_oid: format!("tree-{FIRST}"),
            },
            &ProjectionMaterialization::PreserveGitCommit {
                oid: HEAD.into(),
                parent_oids: vec![FIRST.into()],
                tree_oid: format!("tree-{HEAD}"),
            },
        ]
    );
    assert_eq!(
        agent_projection.visible_paths(),
        ["/README.md", "/src/lib.rs"]
    );

    for (view, paths) in [
        (ViewId::public(), vec!["/README.md"]),
        (ViewId::private(), vec!["/README.md", "/src/lib.rs"]),
    ] {
        let projection = project(&view);
        assert!(!projection.preserves_git_commits(), "{view}");
        assert_eq!(projection.commits.len(), 2, "{view}");
        assert_eq!(projection.visible_paths(), paths, "{view}");
    }
}

#[test]
fn an_agent_request_merge_cannot_carry_paths_outside_the_agent_view() {
    for (changed, touched) in [
        ("/ops/deploy.sh", vec!["/ops/deploy.sh"]),
        ("/src/lib.rs", vec!["/src/lib.rs", "/ops/transient.sh"]),
        ("/src/lib.rs", vec!["/src/lib.rs", "/.scope/repo.json"]),
    ] {
        let mut repo = agent_repo();
        let result = merge(
            &mut repo,
            vec![reviewed_change(changed, Some("change"))],
            agent_origin(vec![native(HEAD, BASE, &touched)]),
        );
        assert!(result.is_err(), "{changed} {touched:?}");
    }
}

#[test]
fn a_request_merge_through_the_full_view_is_not_a_view_origin() {
    let mut repo = agent_repo();
    let mut origin = agent_origin(vec![native(HEAD, BASE, &["/src/lib.rs"])]);
    if let RequestMergeOrigin::View { view, .. } = &mut origin {
        *view = ViewId::private();
    }
    assert!(
        merge(
            &mut repo,
            vec![reviewed_change("/src/lib.rs", Some("change"))],
            origin,
        )
        .is_err()
    );
}

#[test]
fn redacting_public_history_clears_preservation_for_later_merges_in_any_view() {
    let mut repo = agent_repo();
    merge(
        &mut repo,
        vec![reviewed_change("/src/lib.rs", Some("agent code"))],
        agent_origin(vec![native(HEAD, BASE, &["/src/lib.rs"])]),
    )
    .unwrap();
    let mut config = repo.repo_config.clone();
    config.history.rewrites = vec![HistoryRewriteRequest {
        path: "/README.md".into(),
        action: HistoryRewriteAction::RedactPublicHistory,
    }];
    apply_update_with_head(
        &mut repo,
        "4444444444444444444444444444444444444444",
        "redact readme",
        vec![reviewed_change("/.scope/runs/test.yml", Some("name: Test"))],
        None,
        config,
    );
    assert!(matches!(
        &repo.graph.commits[1].origin,
        LogicalCommitOrigin::RequestMerge {
            preserve_commits: false,
            ..
        }
    ));
}
