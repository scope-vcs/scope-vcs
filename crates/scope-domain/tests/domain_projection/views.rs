use super::*;
use scope_domain::{
    error::DomainErrorKind,
    projection::ProjectedCommit,
    projection_views::projected_files,
    repository::collaboration::{RepositoryMember, RepositoryMemberPermissions},
    views::{ViewDefinition, ViewIncludes, ViewReaders, ViewsTransition},
};

fn view_id(value: &str) -> ViewId {
    ViewId::parse(value).unwrap()
}

fn views_with(custom: &[(&str, &str, &[&str])]) -> Views {
    let mut definitions = Vec::<ViewDefinition>::from(Views::builtin());
    definitions.extend(custom.iter().map(|(id, name, includes)| ViewDefinition {
        id: view_id(id),
        name: name.to_string(),
        includes: ViewIncludes::Some(includes.iter().map(|id| view_id(id)).collect()),
        readers: ViewReaders::Assigned,
    }));
    Views::new(definitions).unwrap()
}

fn config_with(views: Views, rules: &[(&str, &str)]) -> RepoConfig {
    let mut config = RepoConfig::with_default_view(ViewId::private());
    config.views = views;
    config.files.rules = rules
        .iter()
        .map(|(path, view)| RepoConfigFileRule {
            path: path.to_string(),
            view: view_id(view),
        })
        .collect();
    config.validate().unwrap();
    config
}

fn update(repo: &mut Repository, config: RepoConfig) -> Result<bool, ReviewedUpdateError> {
    apply_reviewed_config_to_repo(
        repo,
        ReviewedConfigUpdateInput {
            occurred_at_unix: 1_788_700_000,
            author_id: "owner".to_string(),
            config,
        },
    )
}

fn refusal(result: Result<bool, ReviewedUpdateError>) -> String {
    match result {
        Err(ReviewedUpdateError::Domain(error)) => {
            assert_eq!(error.kind, DomainErrorKind::Conflict);
            error.message
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
}

fn member(view: &str) -> RepositoryMember {
    RepositoryMember {
        repo_id: "owner/repo".to_string(),
        user_id: "member".to_string(),
        permissions: RepositoryMemberPermissions {
            can_push: false,
            can_change_file_visibility: false,
            view: view_id(view),
        },
        created_at_unix: 1,
        updated_at_unix: 1,
    }
}

fn repo_with_files() -> Repository {
    let mut repo = published_test_repo(ViewId::private());
    let mut initial = commit(
        "rv1",
        "initial",
        added("/README.md", ViewId::public(), "readme"),
    );
    initial
        .changes
        .push(added("/src/main.rs", ViewId::private(), "main"));
    initial
        .changes
        .push(added("/notes.md", ViewId::private(), "notes"));
    repo.graph.commits.push(initial);
    for (file, content) in [
        ("/README.md", "readme"),
        ("/src/main.rs", "main"),
        ("/notes.md", "notes"),
    ] {
        repo.live_files.insert(path(file), blob(content));
    }
    repo.repo_config = config_with(Views::builtin(), &[("/README.md", "public")]);
    repo.policy
        .add_rule(LabelRule::public(path("/README.md")))
        .unwrap();
    repo
}

fn member_paths(repo: &Repository) -> Vec<String> {
    let access = repo.access_for_user_id("member");
    projected_files(repo, repo.repo_config.views(), &access.view)
        .into_iter()
        .map(|file| file.path.as_str().to_string())
        .collect()
}

fn agent_projection(repo: &Repository) -> Vec<ProjectedCommit> {
    project_graph(
        &repo.graph,
        &repo.visibility_change_sets,
        repo.repo_config.views(),
        &view_id("agent"),
    )
    .commits
}

#[test]
fn a_view_that_starts_including_another_gets_one_boundary_of_the_entering_files() {
    let mut first = commit(
        "rv1",
        "first",
        added("/README.md", ViewId::public(), "readme"),
    );
    first
        .changes
        .push(added("/secret.md", ViewId::private(), "secret"));
    let source = graph(vec![
        first,
        commit("rv2", "second", added("/docs.md", ViewId::public(), "docs")),
    ]);
    let without = views_with(&[("agent", "Agent", &[])]);
    let with_public = views_with(&[("agent", "Agent", &["public"])]);
    let transition = |id: &str, anchor: &str, before: &Views, after: &Views| {
        VisibilityChangeSet::new(
            id.to_string(),
            Some(anchor.to_string()),
            None,
            "owner".to_string(),
            Vec::new(),
            Some(ViewsTransition {
                before: before.clone(),
                after: after.clone(),
            }),
        )
        .unwrap()
    };
    let sets = vec![transition("include", "rv1", &without, &with_public)];
    let projection = project_graph(&source, &sets, &with_public, &view_id("agent"));

    let boundaries = projection
        .commits
        .iter()
        .filter(|commit| commit.visibility_change_set_id.is_some())
        .collect::<Vec<_>>();
    assert_eq!(boundaries.len(), 1);
    assert_eq!(boundaries[0].message, "Projection view boundary");
    assert_eq!(
        boundaries[0].visibility_change_set_id.as_deref(),
        Some("include")
    );
    assert_eq!(
        boundaries[0]
            .changes
            .iter()
            .map(|change| (change.path.as_str(), change.new_content.clone()))
            .collect::<Vec<_>>(),
        [("/README.md", Some(blob("readme")))]
    );
    assert_eq!(projection.commits.len(), 2);
    assert_eq!(projection.visible_paths(), ["/README.md", "/docs.md"]);

    let mut later = sets.clone();
    later.push(transition("exclude", "rv2", &with_public, &without));
    let projection = project_graph(&source, &later, &without, &view_id("agent"));
    let leaving = projection.commits.last().unwrap();
    assert_eq!(leaving.visibility_change_set_id.as_deref(), Some("exclude"));
    assert!(
        leaving
            .changes
            .iter()
            .all(|change| change.new_content.is_none())
    );
    assert!(projection.visible_paths().is_empty());

    for view in [ViewId::public(), ViewId::private()] {
        assert_eq!(
            project_graph(&source, &sets, &with_public, &view),
            project_graph(&source, &[], &Views::builtin(), &view)
        );
    }
}

#[test]
fn an_agent_member_reads_exactly_the_public_and_agent_files() {
    let mut repo = repo_with_files();
    let agent_views = views_with(&[("agent", "Agent", &["public"])]);
    assert!(
        update(
            &mut repo,
            config_with(
                agent_views.clone(),
                &[("/README.md", "public"), ("/src/**", "agent")]
            )
        )
        .unwrap()
    );
    let set = repo.visibility_change_sets.last().unwrap();
    assert_eq!(set.views.as_ref().unwrap().after, agent_views);
    assert_eq!(set.changes.len(), 1);
    assert_eq!(set.changes[0].new_label, view_id("agent"));

    let assigned = member("agent");
    assigned.permissions.validate(&agent_views).unwrap();
    let mut pushing = assigned.permissions.clone();
    pushing.can_push = true;
    pushing.validate(&agent_views).unwrap();
    let mut relabelling = assigned.permissions.clone();
    relabelling.can_change_file_visibility = true;
    assert!(relabelling.validate(&agent_views).is_err());
    repo.collaboration.members.push(assigned);

    let access = repo.access_for_user_id("member");
    assert_eq!(access.view, view_id("agent"));
    assert!(repo.can_read_view(&access, &ViewId::public()));
    assert!(repo.can_read_view(&access, &view_id("agent")));
    assert!(!repo.can_read_view(&access, &ViewId::private()));
    assert_eq!(member_paths(&repo), ["/README.md", "/src/main.rs"]);
}

#[test]
fn renaming_a_view_records_no_change_and_keeps_every_projected_commit() {
    let mut repo = repo_with_files();
    update(
        &mut repo,
        config_with(
            views_with(&[("agent", "Agent", &["public"])]),
            &[("/README.md", "public"), ("/src/**", "agent")],
        ),
    )
    .unwrap();
    let sets = repo.visibility_change_sets.len();
    let before = agent_projection(&repo);

    assert!(
        update(
            &mut repo,
            config_with(
                views_with(&[("agent", "Coding agents", &["public"])]),
                &[("/README.md", "public"), ("/src/**", "agent")],
            ),
        )
        .unwrap()
    );
    assert_eq!(repo.visibility_change_sets.len(), sets);
    assert_eq!(agent_projection(&repo), before);
    assert_eq!(
        repo.repo_config.views().display_name(&view_id("agent")),
        "Coding agents"
    );
}

#[test]
fn including_a_new_view_appends_one_boundary_commit() {
    let mut repo = repo_with_files();
    let rules = [
        ("/README.md", "public"),
        ("/src/**", "agent"),
        ("/notes.md", "docs"),
    ];
    update(
        &mut repo,
        config_with(
            views_with(&[("agent", "Agent", &["public"]), ("docs", "Docs", &[])]),
            &rules,
        ),
    )
    .unwrap();
    let before = agent_projection(&repo);

    update(
        &mut repo,
        config_with(
            views_with(&[
                ("agent", "Agent", &["public", "docs"]),
                ("docs", "Docs", &[]),
            ]),
            &rules,
        ),
    )
    .unwrap();
    let after = agent_projection(&repo);
    assert_eq!(after[..before.len()], before[..]);
    assert_eq!(after.len(), before.len() + 1);
    let boundary = after.last().unwrap();
    assert_eq!(boundary.message, "Projection view boundary");
    assert_eq!(
        boundary.parent_projected_id,
        before.last().map(|commit| commit.projected_id.clone())
    );
    assert_eq!(
        boundary
            .changes
            .iter()
            .map(|change| change.path.as_str())
            .collect::<Vec<_>>(),
        ["/notes.md"]
    );
}

#[test]
fn removing_a_view_waits_until_its_members_and_files_are_reassigned() {
    let mut repo = repo_with_files();
    let agent_views = views_with(&[("agent", "Agent", &["public"])]);
    update(
        &mut repo,
        config_with(
            agent_views.clone(),
            &[("/README.md", "public"), ("/src/**", "agent")],
        ),
    )
    .unwrap();
    repo.collaboration.members.push(member("agent"));
    let removed = config_with(Views::builtin(), &[("/README.md", "public")]);

    assert!(refusal(update(&mut repo, removed.clone())).contains("members"));
    repo.collaboration.members[0].permissions.view = ViewId::public();
    assert!(refusal(update(&mut repo, removed.clone())).contains("files"));

    let mut still_named = removed.clone();
    still_named.files.rules.push(RepoConfigFileRule {
        path: "/docs/**".to_string(),
        view: view_id("agent"),
    });
    update(
        &mut repo,
        config_with(agent_views, &[("/README.md", "public")]),
    )
    .unwrap();
    assert!(refusal(update(&mut repo, still_named)).contains("rule"));
    let mut still_default = removed.clone();
    still_default.files.default = view_id("agent");
    assert!(refusal(update(&mut repo, still_default)).contains("default"));

    assert!(update(&mut repo, removed).unwrap());
    let set = repo.visibility_change_sets.last().unwrap();
    assert_eq!(set.views.as_ref().unwrap().after, Views::builtin());
    assert_eq!(member_paths(&repo), ["/README.md"]);
}

#[test]
fn a_relabel_hidden_from_its_push_still_enters_when_the_view_starts_including_its_label() {
    let mut first = commit(
        "rv1",
        "first",
        added("/README.md", ViewId::public(), "readme"),
    );
    first
        .changes
        .push(added("/tool.rs", ViewId::private(), "tool"));
    let source = graph(vec![
        first,
        commit(
            "rv2",
            "second",
            added("/tool.rs", view_id("ops"), "tool v2"),
        ),
    ]);
    let before = views_with(&[("ops", "Ops", &[]), ("agent", "Agent", &["public"])]);
    let after = views_with(&[("ops", "Ops", &[]), ("agent", "Agent", &["public", "ops"])]);
    let push = VisibilityChangeSet::new(
        "relabel".to_string(),
        Some("rv2".to_string()),
        Some("rv2".to_string()),
        "owner".to_string(),
        vec![scope_domain::visibility_changes::VisibilityChange {
            path: path("/tool.rs"),
            old_label: ViewId::private(),
            new_label: view_id("ops"),
            current_content: Some(blob("tool v2")),
        }],
        Some(ViewsTransition {
            before: before.clone(),
            after: after.clone(),
        }),
    )
    .unwrap();

    let projection = project_graph(&source, &[push], &before, &view_id("agent"));

    let boundary = projection
        .commits
        .iter()
        .find(|commit| commit.visibility_change_set_id.as_deref() == Some("relabel"))
        .expect("the push's boundary commit");
    assert_eq!(
        boundary
            .changes
            .iter()
            .map(|change| (change.path.as_str(), change.new_content.clone()))
            .collect::<Vec<_>>(),
        [("/tool.rs", Some(blob("tool v2")))]
    );
    assert_eq!(projection.visible_paths(), ["/README.md", "/tool.rs"]);
}

#[test]
fn a_views_transition_survives_a_public_history_redaction() {
    let mut repo = repo_with_files();
    let separate = views_with(&[("ops", "Ops", &[]), ("agent", "Agent", &["public"])]);
    update(
        &mut repo,
        config_with(separate, &[("/README.md", "public")]),
    )
    .unwrap();
    let joined = views_with(&[("ops", "Ops", &[]), ("agent", "Agent", &["public", "ops"])]);
    update(
        &mut repo,
        config_with(joined.clone(), &[("/README.md", "public")]),
    )
    .unwrap();
    let transitions = |repo: &Repository| {
        repo.visibility_change_sets
            .iter()
            .filter(|set| set.views.is_some())
            .count()
    };
    assert_eq!(transitions(&repo), 2);
    assert!(
        repo.visibility_change_sets
            .last()
            .is_some_and(|set| set.changes.is_empty() && set.views.is_some())
    );

    let mut redacting = config_with(joined.clone(), &[]);
    redacting.history.rewrites = vec![scope_domain::repo_config::HistoryRewriteRequest {
        path: "/README.md".to_string(),
        action: scope_domain::repo_config::HistoryRewriteAction::RedactPublicHistory,
    }];
    update(&mut repo, redacting).unwrap();

    assert_eq!(transitions(&repo), 2);
    assert_eq!(
        agent_projection(&repo).len(),
        project_graph(
            &repo.graph,
            &repo.visibility_change_sets,
            &Views::builtin(),
            &view_id("agent")
        )
        .commits
        .len()
    );
}
