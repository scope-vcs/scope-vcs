use super::*;
use crate::db::requests::tests::{postgres_store, start_public_request};

#[tokio::test]
async fn pool_bundle_backfill_serializes_replicas_recovers_crashes_and_reports_failures() {
    let store = postgres_store();
    start_public_request(&store).await;
    let repositories = store.repositories();
    let request = store
        .requests()
        .request_by_id("req_1")
        .await
        .unwrap()
        .unwrap();
    let full_pack = GitPoolPack {
        id: "full-only".into(),
        owner: scope_domain::repository::git::GitPoolPackOwner::Request {
            request_id: request.id,
            scope: scope_domain::repository::git::RequestPackScope::Full,
        },
        head_oid: request.head_oid,
        prerequisites: vec![],
        storage: scope_domain::repository::git::GitPoolPackStorage::Inline { sealed: vec![1] },
    };
    register_git_pool_pack(store.db.as_ref(), "owner/repo", &full_pack, 1)
        .await
        .unwrap();
    let before = report(store.db.as_ref()).await.unwrap();
    assert_eq!(before.total.needing_backfill, 1);
    let claim = repositories
        .claim_pool_bundle_backfill("first", 10)
        .await
        .unwrap()
        .unwrap();
    assert!(
        repositories
            .claim_pool_bundle_backfill("second", 1000)
            .await
            .unwrap()
            .is_none()
    );
    drop(claim);
    assert!(
        repositories
            .claim_pool_bundle_backfill("second", 11)
            .await
            .unwrap()
            .is_none()
    );
    let claim = repositories
        .claim_pool_bundle_backfill("second", 610)
        .await
        .unwrap()
        .unwrap();
    repositories
        .fail_pool_bundle_backfill(&claim, "bundle\0 missing", 610)
        .await
        .unwrap();
    repositories
        .release_pool_bundle_backfill(claim, 610)
        .await
        .unwrap();
    let failed = report(store.db.as_ref()).await.unwrap();
    assert_eq!(
        failed.total,
        BundleBackfillCounts {
            remaining: 1,
            needing_backfill: 0,
            done: 0,
            failed: 1
        }
    );
    assert_eq!(failed.repositories[0].counts, failed.total);
    assert_eq!(failed.failures[0].reason, "bundle missing");
    assert!(
        repositories
            .claim_pool_bundle_backfill("third", 640)
            .await
            .unwrap()
            .is_none()
    );
    let claim = repositories
        .claim_pool_bundle_backfill("third", 87010)
        .await
        .unwrap()
        .unwrap();
    let request = store
        .requests()
        .request_by_id(&claim.source.request_id)
        .await
        .unwrap()
        .unwrap();
    let repo = repositories
        .repository_record(&claim.source.repo_id)
        .await
        .unwrap()
        .unwrap();
    let config = repositories
        .git_push_state(&repo.owner_handle, &repo.name, &repo.owner_user_id)
        .await
        .unwrap()
        .unwrap()
        .repo_config;
    let pack = GitPoolPack {
        id: "backfilled".into(),
        owner: scope_domain::repository::git::GitPoolPackOwner::Request {
            request_id: request.id,
            scope: scope_domain::repository::git::RequestPackScope::View(request.view),
        },
        head_oid: claim.source.head_oid.clone(),
        prerequisites: vec![],
        storage: scope_domain::repository::git::GitPoolPackStorage::Inline { sealed: vec![1] },
    };
    let verification = BundleBackfillVerification {
        restored: scope_domain::repository::bundle_backfill::PoolRestoreFacts {
            head_present: true,
            old_head_present: true,
            connected: true,
            within_reader_limits: true,
        }
        .verify()
        .unwrap(),
        pack_ids: vec![pack.id.clone()],
        request_pack_ids: vec![pack.id.clone()],
        reader_request_capacities: [(claim.source.request_id.clone(), 16)]
            .into_iter()
            .collect(),
    };
    assert!(
        repositories
            .complete_pool_bundle_backfill(
                &claim,
                &repo.incarnation(),
                &config.object_generation(),
                Some(&full_pack),
                &verification,
                87010,
            )
            .await
            .is_err()
    );
    let held = store.db.begin().await.unwrap();
    acquire_aggregate_lock(&held, "request", &claim.source.request_id)
        .await
        .unwrap();
    let blocker = super::super::locks::backend_pid(&held).await;
    let repositories_for_completion = repositories.clone();
    let incarnation = repo.incarnation();
    let generation = config.object_generation();
    let completion = tokio::spawn(async move {
        let result = repositories_for_completion
            .complete_pool_bundle_backfill(
                &claim,
                &incarnation,
                &generation,
                Some(&pack),
                &verification,
                87010,
            )
            .await;
        (claim, pack, verification, result)
    });
    super::super::locks::wait_for_transaction_waiter(&store, blocker).await;
    held.commit().await.unwrap();
    let (claim, pack, verification, completed) = completion.await.unwrap();
    assert!(completed.unwrap());
    assert!(
        !repositories
            .complete_pool_bundle_backfill(
                &claim,
                &repo.incarnation(),
                &config.object_generation(),
                Some(&pack),
                &verification,
                87010
            )
            .await
            .unwrap()
    );
    repositories
        .release_pool_bundle_backfill(claim, 87010)
        .await
        .unwrap();
    assert!(
        repositories
            .claim_pool_bundle_backfill("fourth", 87040)
            .await
            .unwrap()
            .is_none()
    );
    let after = report(store.db.as_ref()).await.unwrap();
    assert_eq!(
        after.total,
        BundleBackfillCounts {
            remaining: 0,
            needing_backfill: 0,
            done: 1,
            failed: 0
        }
    );
    assert!(after.failures.is_empty());
}

#[tokio::test]
async fn pool_bundle_backfill_maintenance_report_groups_repositories_without_writing() {
    let store = postgres_store();
    start_public_request(&store).await;
    let owner = scope_domain::account::UserAccount {
        id: "user_owner".into(),
        handle: "owner".into(),
        email: "owner@example.com".into(),
        email_verified: true,
    };
    let mut repo = scope_domain::repository::Repository::new(
        &owner,
        "another",
        scope_domain::views::ViewId::public(),
        "repoi_another",
    )
    .unwrap();
    repo.record.lifecycle_state = scope_domain::repository::RepoLifecycleState::Ready;
    super::super::repository_rows::insert_repository(store.db.as_ref(), &repo, 1)
        .await
        .unwrap();
    let mut input = crate::db::requests::tests::public_start_input();
    input.id = "req_2".into();
    input.repo_id = repo.record.id;
    input.event_id = "event_2".into();
    store.requests().start_request(input).await.unwrap();
    let mut upload = crate::db::requests::tests::public_upload_input();
    upload.request_id = "req_2".into();
    store
        .requests()
        .record_working_request_upload(upload)
        .await
        .unwrap();
    let schema = content_fences::current_schema(store.db.as_ref())
        .await
        .unwrap();
    let mut url = url::Url::parse(store.postgres_database_url.as_deref().unwrap()).unwrap();
    url.query_pairs_mut()
        .append_pair("options[search_path]", &schema)
        .append_pair("options[default_transaction_read_only]", "on");
    let report = pool_bundle_backfill_report_for_maintenance(url.into())
        .await
        .unwrap();
    assert_eq!(report.total.remaining, 2);
    assert_eq!(report.repositories.len(), 2);
    for repo in report.repositories {
        assert_eq!(repo.counts.needing_backfill, 1);
    }
    let lease = store
        .db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT token, available_at_unix FROM scope_pool_bundle_backfill_lease".to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lease.try_get::<Option<String>>("", "token").unwrap(), None);
    assert_eq!(lease.try_get::<i64>("", "available_at_unix").unwrap(), 0);
}
