use super::push_paths::{PUSH_PATHS, push};
use super::*;
use scope_domain::policy::LabelRule;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

const DELETED_HEAD: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

async fn stored_live_file_views(store: &crate::db::MetadataStore) -> Vec<(String, Option<String>)> {
    store
        .db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT path, view FROM scope_live_files WHERE repo_id = 'owner/repo' ORDER BY path COLLATE \"C\"",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get("", "path").unwrap(),
                row.try_get("", "view").unwrap(),
            )
        })
        .collect()
}

async fn stored_policy_rules(store: &crate::db::MetadataStore) -> serde_json::Value {
    store
        .db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT policy->'rules' AS rules FROM scope_repositories WHERE id = 'owner/repo'",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "rules")
        .unwrap()
}

async fn deletion(store: &crate::db::MetadataStore, path: &str) -> MergePreparation {
    let repo = store
        .repositories()
        .repository_for_tests("owner/repo")
        .await
        .unwrap()
        .unwrap();
    let mut update = reviewed_update(
        &repo,
        DELETED_HEAD,
        3,
        Some(MERGED_HEAD),
        path,
        source_blob("deleted"),
    );
    update.changes[0].content = None;
    stage_segment(store, &update.git_pack_span.segment).await;
    MergePreparation {
        expected_git_frontier: repo.git_head.as_ref().unwrap().frontier(),
        expected_repo_change_version: repo.record.change_version,
        workflow_catalog: RepositoryWorkflowCatalog::captured(
            "owner/repo",
            DELETED_HEAD,
            repo.record.change_version + 1,
            Vec::new(),
        )
        .unwrap(),
        update,
    }
}

#[tokio::test]
async fn main_pushes_keep_file_rules_on_live_files_instead_of_the_repository_policy() {
    let readme = ScopePath::parse("/README.md").unwrap();
    for path in PUSH_PATHS {
        let store = merge_store().await;
        let prepared = merge_preparation(&store).await;
        push(&store, path, prepared, Vec::new()).await.unwrap();

        let repo = store
            .repositories()
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .unwrap();
        let rules = repo.policy.rules().to_vec();
        assert!(rules.contains(&LabelRule {
            view: repo.repo_config.label_for_path(&readme),
            path: readme.clone(),
        }));
        assert_eq!(
            stored_live_file_views(&store).await,
            rules
                .iter()
                .map(|rule| (rule.path.to_string(), Some(rule.view.to_string())))
                .collect::<Vec<_>>()
        );
        assert_eq!(stored_policy_rules(&store).await, serde_json::json!([]));

        let deleted = deletion(&store, "/README.md").await;
        push(&store, path, deleted, Vec::new()).await.unwrap();

        let repo = store
            .repositories()
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .unwrap();
        assert!(repo.policy.rules().iter().all(|rule| rule.path != readme));
        assert!(
            stored_live_file_views(&store)
                .await
                .iter()
                .all(|(path, _)| path != "/README.md")
        );
        assert_eq!(stored_policy_rules(&store).await, serde_json::json!([]));
    }
}

async fn live_file_versions(store: &crate::db::MetadataStore) -> Vec<(String, String)> {
    store
        .db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT path, xmin::text AS version FROM scope_live_files
             WHERE repo_id = 'owner/repo' ORDER BY path COLLATE \"C\"",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get("", "path").unwrap(),
                row.try_get("", "version").unwrap(),
            )
        })
        .collect()
}

#[tokio::test]
async fn relabelling_one_file_rewrites_only_its_row() {
    let store = merge_store().await;
    let prepared = merge_preparation(&store).await;
    push(&store, PUSH_PATHS[0], prepared, Vec::new())
        .await
        .unwrap();
    let before = live_file_versions(&store).await;

    store
        .repositories()
        .mutate_repository_for_tests("owner/repo", |repo| {
            repo.policy
                .add_rule(LabelRule::private(ScopePath::parse("/README.md").unwrap()))
                .unwrap();
            repo.bump_content_version();
        })
        .await
        .unwrap();

    let after = live_file_versions(&store).await;
    let rewritten = before
        .iter()
        .zip(&after)
        .filter(|(before, after)| before != after)
        .map(|(_, (path, _))| path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(rewritten, ["/README.md"]);
    assert!(
        stored_live_file_views(&store)
            .await
            .contains(&("/README.md".to_string(), Some("private".to_string())))
    );
}
