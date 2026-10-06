use super::*;
use crate::db::{
    MetadataStore, RepositoryHistoryQuery, TestDatabaseTarget,
    entities::projection_read_model::Model as ReadModel,
};
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    history::{HistoryFeed, history_view},
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin, project_graph},
    repository::{
        RepoLifecycleState, Repository,
        collaboration::{RepositoryMember, RepositoryMemberPermissions},
    },
    views::{ViewDefinition, ViewIncludes, ViewReaders, ViewsTransition},
    visibility_changes::{VisibilityChange, VisibilityChangeSet},
};
use sea_orm::{DatabaseBackend, Statement};

fn view(id: &str) -> ViewId {
    ViewId::parse(id).unwrap()
}

fn views_with(custom: &[(&str, &[&str])]) -> Views {
    let mut definitions = Vec::<ViewDefinition>::from(Views::builtin());
    for (id, includes) in custom {
        definitions.push(ViewDefinition {
            id: view(id),
            name: id.to_uppercase(),
            includes: ViewIncludes::Some(includes.iter().map(|id| view(id)).collect()),
            readers: ViewReaders::Assigned,
        });
    }
    Views::new(definitions).unwrap()
}

fn blob(seed: u64) -> SourceBlob {
    SourceBlob {
        content_ref: ContentRef::git_bundle_sha256(format!("bundle-{seed}")),
        sha256: format!("hash-{seed}"),
        git_oid: format!("{seed:040x}"),
        git_file_mode: "100644".into(),
        size_bytes: 10,
    }
}

fn commit(id: &str, files: &[(&str, &str, Option<u64>, u64)]) -> LogicalCommit {
    LogicalCommit {
        occurred_at_unix: Some(1_700_000_000),
        id: id.into(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: format!("{:040x}", files[0].3 + 1_000),
        },
        author_id: "views_owner".into(),
        message: format!("Commit {id}"),
        changes: files
            .iter()
            .map(|(path, label, old, new)| FileChange {
                path: ScopePath::parse(*path).unwrap(),
                old_content: old.map(blob),
                new_content: Some(blob(*new)),
                label: view(label),
            })
            .collect(),
    }
}

fn views_change(
    id: &str,
    anchor: &str,
    before: Views,
    after: Views,
    changes: Vec<VisibilityChange>,
) -> VisibilityChangeSet {
    VisibilityChangeSet::new(
        id.into(),
        Some(anchor.into()),
        None,
        "views_owner".into(),
        changes,
        Some(ViewsTransition { before, after }),
    )
    .unwrap()
}

fn store_and_repo(member_view: &str) -> (MetadataStore, Repository) {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let owner = UserAccount {
        id: "views_owner".into(),
        handle: "owner".into(),
        email: "views@example.com".into(),
        email_verified: true,
    };
    let mut repo = Repository::new(&owner, "views", ViewId::private(), "repoi_views").unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    repo.collaboration.members.push(RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: "views_member".into(),
        permissions: RepositoryMemberPermissions {
            can_push: false,
            can_change_file_visibility: false,
            view: view(member_view),
        },
        created_at_unix: 1,
        updated_at_unix: 1,
    });
    (store, repo)
}

fn record(repo: &mut Repository, commits: Vec<LogicalCommit>, sets: Vec<VisibilityChangeSet>) {
    for commit in commits {
        for change in &commit.changes {
            repo.live_files
                .insert(change.path.clone(), change.new_content.clone().unwrap());
        }
        repo.graph.commits.push(commit);
    }
    for set in sets {
        if let Some(transition) = &set.views {
            repo.repo_config.views = transition.after.clone();
        }
        repo.visibility_change_sets.push(set);
    }
    repo.bump_content_version();
}

async fn fold(store: &MetadataStore, repo: &Repository) {
    fold_live_projection_read_models(
        store.db.as_ref(),
        &repo.record.id,
        repo.record.content_version,
    )
    .await
    .unwrap();
}

async fn read_model(store: &MetadataStore, repo: &Repository, id: &ViewId) -> Option<ReadModel> {
    live_projection_read_model(
        store.db.as_ref(),
        &repo.record.id,
        repo.record.content_version,
        id,
    )
    .await
    .unwrap()
}

async fn stored_views(store: &MetadataStore, repo: &Repository) -> Vec<String> {
    store
        .db
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT DISTINCT audience FROM scope_repository_history_entries WHERE repo_id=$1 \
             UNION SELECT audience FROM scope_projection_read_models WHERE repo_id=$1 ORDER BY 1",
            [repo.record.id.clone().into()],
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "audience").unwrap())
        .collect()
}

#[tokio::test]
async fn an_include_change_folds_into_one_boundary_commit_on_top_of_the_resumed_read_models() {
    let (store, mut repo) = store_and_repo("agent");
    let agent_alone = views_with(&[("agent", &[])]);
    let agent_with_public = views_with(&[("agent", &["public"])]);
    record(
        &mut repo,
        vec![
            commit(
                "c0",
                &[
                    ("/README.md", "public", None, 1),
                    ("/src/main.rs", "private", None, 2),
                    ("/secret.md", "private", None, 3),
                ],
            ),
            commit("c1", &[("/README.md", "public", Some(1), 4)]),
        ],
        vec![views_change(
            "vchg_agent",
            "c0",
            Views::builtin(),
            agent_alone.clone(),
            vec![VisibilityChange {
                path: ScopePath::parse("/src/main.rs").unwrap(),
                old_label: ViewId::private(),
                new_label: view("agent"),
                current_content: Some(blob(2)),
            }],
        )],
    );
    store
        .repositories()
        .replace_repository_for_tests(repo.clone())
        .await
        .unwrap();
    fold(&store, &repo).await;
    let folded = read_model(&store, &repo, &view("agent")).await.unwrap();

    record(
        &mut repo,
        vec![commit("c2", &[("/docs.md", "public", None, 5)])],
        vec![views_change(
            "vchg_include",
            "c1",
            agent_alone,
            agent_with_public.clone(),
            Vec::new(),
        )],
    );
    store
        .repositories()
        .replace_repository_for_tests(repo.clone())
        .await
        .unwrap();
    store
        .db
        .execute_unprepared("DELETE FROM scope_file_changes WHERE commit_id <> 'c2'")
        .await
        .unwrap();
    fold(&store, &repo).await;

    let agent = read_model(&store, &repo, &view("agent")).await.unwrap();
    assert_eq!(agent.projected_commits, folded.projected_commits + 2);
    for id in [ViewId::private(), ViewId::public(), view("agent")] {
        let projection = project_graph(
            &repo.graph,
            &repo.visibility_change_sets,
            &agent_with_public,
            &id,
        );
        let row = read_model(&store, &repo, &id).await.unwrap();
        assert_eq!(
            row.head_oid,
            scope_git::projection_head_oid(&projection).unwrap(),
            "{id}"
        );
        assert_eq!(row.projected_commits as usize, projection.commits.len());
        assert_eq!(
            serde_json::from_value::<Views>(row.views).unwrap(),
            agent_with_public
        );
        let page = store
            .repositories()
            .repository_history_page(RepositoryHistoryQuery {
                incarnation: &repo.incarnation(),
                change_version: repo.record.change_version,
                view: &id,
                feed: HistoryFeed::All,
                before: None,
                entry_source_id: None,
                limit: 50,
            })
            .await
            .unwrap();
        assert_eq!(
            page.view.entries,
            history_view(
                &repo.graph,
                &repo.visibility_change_sets,
                &agent_with_public,
                &id
            )
            .entries
        );
    }
    let boundaries = project_graph(
        &repo.graph,
        &repo.visibility_change_sets,
        &agent_with_public,
        &view("agent"),
    )
    .commits
    .into_iter()
    .filter(|commit| commit.visibility_change_set_id.as_deref() == Some("vchg_include"))
    .collect::<Vec<_>>();
    assert_eq!(boundaries.len(), 1);
    assert_eq!(
        boundaries[0]
            .changes
            .iter()
            .map(|change| change.path.as_str())
            .collect::<Vec<_>>(),
        ["/README.md"]
    );
    let files = load_live_projection_files_for_view(
        store.db.as_ref(),
        &repo.record.id,
        repo.record.content_version,
        &view("agent"),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        ["/README.md", "/docs.md", "/src/main.rs"].into()
    );
}

#[tokio::test]
async fn unassigned_views_build_on_first_read_and_removed_views_lose_their_rows() {
    let (store, mut repo) = store_and_repo("private");
    let with_docs = views_with(&[("docs", &["public"])]);
    record(
        &mut repo,
        vec![commit(
            "c0",
            &[
                ("/README.md", "public", None, 1),
                ("/guide.md", "private", None, 2),
            ],
        )],
        vec![views_change(
            "vchg_docs",
            "c0",
            Views::builtin(),
            with_docs.clone(),
            Vec::new(),
        )],
    );
    store
        .repositories()
        .replace_repository_for_tests(repo.clone())
        .await
        .unwrap();
    fold(&store, &repo).await;
    assert_eq!(stored_views(&store, &repo).await, ["private", "public"]);

    store
        .repositories()
        .ensure_live_projection_read_models(&repo.incarnation(), &view("docs"))
        .await
        .unwrap();
    let docs = read_model(&store, &repo, &view("docs")).await.unwrap();
    assert_eq!(docs.file_count, 1);
    assert_eq!(
        stored_views(&store, &repo).await,
        ["docs", "private", "public"]
    );
    let missing = store
        .repositories()
        .ensure_live_projection_read_models(&repo.incarnation(), &view("absent"))
        .await
        .unwrap_err();
    assert_eq!(missing.message, "repository view not found");

    record(
        &mut repo,
        vec![commit("c1", &[("/guide.md", "private", Some(2), 3)])],
        vec![views_change(
            "vchg_drop_docs",
            "c1",
            with_docs,
            Views::builtin(),
            Vec::new(),
        )],
    );
    store
        .repositories()
        .replace_repository_for_tests(repo.clone())
        .await
        .unwrap();
    fold(&store, &repo).await;
    assert_eq!(stored_views(&store, &repo).await, ["private", "public"]);
    assert!(
        read_model(&store, &repo, &ViewId::private())
            .await
            .is_some()
    );
}
