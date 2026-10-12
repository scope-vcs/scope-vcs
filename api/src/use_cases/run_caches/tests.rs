use super::{
    RunCacheStorage,
    reconciliation::{DELETE_CONCURRENCY, RETRY_SECONDS, reconcile_at},
};
use crate::{
    app::router, auth::tokens::machine_token_hash, persistence::unix_now, state::AppState,
};
use axum::{
    body::{Body, to_bytes},
    extract::State,
    http::{Method, Request, Response, StatusCode, header::AUTHORIZATION},
    routing::any,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scope_api_contract::{
    AttemptCacheKey, CommitAttemptCacheUploadRequest, PrepareAttemptCacheUploadRequest,
    PrepareAttemptCacheUploadResponse, RestoreAttemptCacheRequest, RestoreAttemptCacheResponse,
    routes,
};
use scope_domain::{
    account::UserAccount,
    content_ref::ContentRef,
    repository::{RepoLifecycleState, Repository},
    runs::{
        cache::definition::{CacheKeyInputs, WorkflowCache},
        run::Run,
        source::{RunSource, RunTrigger},
        workflow::{
            definition::{
                CompiledWorkflow, ContainerSpec, WorkflowJob, WorkflowJobId, WorkflowStep,
                WorkflowTriggers,
            },
            identity::{WorkflowIdentity, WorkflowPath},
            revision::WorkflowRevision,
        },
    },
    views::ViewId,
};
use scope_postgres::db::CatalogFixture;
use scope_storage::S3Settings;
use sha2::{Digest as _, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt as _;

const ATTEMPT_ID: &str = "attempt-run-cache";
const ATTEMPT_TOKEN: &str = "scope_attempt_run_cache";

struct StoredObject {
    bytes: Vec<u8>,
    checksum: String,
}

#[derive(Clone, Default)]
struct FakeObjectStore {
    objects: Arc<Mutex<HashMap<String, StoredObject>>>,
    deletes: Arc<AtomicUsize>,
    active_deletes: Arc<AtomicUsize>,
    peak_deletes: Arc<AtomicUsize>,
    fail_deletes: Arc<AtomicBool>,
    hang_deletes: Arc<AtomicBool>,
}

struct Fixture {
    state: AppState,
    objects: FakeObjectStore,
    repository_id: String,
    now: u64,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        let objects = FakeObjectStore::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = axum::Router::new()
            .fallback(any(fake_object_request))
            .with_state(objects.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut settings = S3Settings::new(
            endpoint,
            "scope-run-cache".to_string(),
            "us-east-1".to_string(),
            "access".to_string(),
            "secret".to_string(),
        );
        settings.force_path_style = true;
        let mut state = AppState::test_state();
        state.run_caches = RunCacheStorage::new(settings, "test-local".to_string()).unwrap();
        let repository_id = seed_repository(&state);
        Self {
            state,
            objects,
            repository_id,
            now: unix_now().unwrap(),
            server,
        }
    }

    async fn upload(&self, index: usize, created_at: u64, committed: bool) {
        let digest = format!("{index:064x}");
        let upload_id = format!("upload-{index}");
        let caches = self.state.metadata.caches();
        caches
            .prepare_upload(
                &self.repository_id,
                &digest,
                &"f".repeat(64),
                &digest,
                1,
                "test-local",
                &upload_id,
                created_at,
            )
            .await
            .unwrap();
        if committed {
            caches
                .commit_upload(&upload_id, created_at + 1)
                .await
                .unwrap();
        }
    }

    fn deletes(&self) -> usize {
        self.objects.deletes.load(Ordering::SeqCst)
    }
}

async fn fake_object_request(
    State(store): State<FakeObjectStore>,
    request: Request<Body>,
) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let key = parts.uri.path().to_string();
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    let status = match parts.method {
        Method::PUT => {
            let checksum = header("x-amz-meta-scope-sha256");
            let checksum_base64 = header("x-amz-checksum-sha256");
            let bytes = to_bytes(body, 1024 * 1024).await.unwrap().to_vec();
            let digest = Sha256::digest(&bytes);
            if checksum.as_deref() != Some(hex::encode(digest).as_str())
                || checksum_base64.as_deref() != Some(BASE64.encode(digest).as_str())
            {
                StatusCode::BAD_REQUEST
            } else {
                store.objects.lock().unwrap().insert(
                    key,
                    StoredObject {
                        bytes,
                        checksum: hex::encode(digest),
                    },
                );
                StatusCode::OK
            }
        }
        Method::HEAD | Method::GET => {
            let objects = store.objects.lock().unwrap();
            let Some(StoredObject { bytes, checksum }) = objects.get(&key) else {
                return status_response(StatusCode::NOT_FOUND);
            };
            let body = if parts.method == Method::GET {
                Body::from(bytes.clone())
            } else {
                Body::empty()
            };
            return Response::builder()
                .header(reqwest::header::CONTENT_LENGTH, bytes.len())
                .header("x-amz-meta-scope-sha256", checksum)
                .body(body)
                .unwrap();
        }
        Method::DELETE => {
            store.deletes.fetch_add(1, Ordering::SeqCst);
            if store.hang_deletes.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            let current = store.active_deletes.fetch_add(1, Ordering::SeqCst) + 1;
            store.peak_deletes.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(5)).await;
            store.active_deletes.fetch_sub(1, Ordering::SeqCst);
            if store.fail_deletes.load(Ordering::SeqCst) {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::NO_CONTENT
            }
        }
        _ => StatusCode::METHOD_NOT_ALLOWED,
    };
    status_response(status)
}

fn status_response(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .unwrap()
}

fn seed_repository(state: &AppState) -> String {
    let owner = UserAccount {
        id: "user_run_cache".to_string(),
        handle: "run-cache".to_string(),
        email: "run-cache@example.com".to_string(),
        email_verified: true,
    };
    let mut repository =
        Repository::new(&owner, "caches", ViewId::private(), "repoi_test").unwrap();
    repository.record.lifecycle_state = RepoLifecycleState::Ready;
    let repository_id = repository.record.id.clone();
    let mut catalog = CatalogFixture::default();
    catalog.native_runs_accounts.insert(owner.id.clone());
    catalog.users.insert(owner.id.clone(), owner);
    catalog
        .repositories
        .insert(repository_id.clone(), repository);
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(catalog)
        .unwrap();
    repository_id
}

async fn dispatch_attempt(fixture: &Fixture) {
    let workflow = WorkflowIdentity::new(
        fixture.repository_id.as_str(),
        WorkflowPath::parse("/.scope/runs/caches.yml").unwrap(),
    )
    .unwrap();
    let definition = CompiledWorkflow::new(
        "Caches",
        WorkflowTriggers::new(true, false, false).unwrap(),
        vec![
            WorkflowJob::new(
                WorkflowJobId::parse("checks").unwrap(),
                vec![],
                ContainerSpec::new(format!("alpine@sha256:{}", "a".repeat(64))).unwrap(),
                600,
                vec![
                    WorkflowCache::new(
                        "cargo",
                        "/scope/cache/cargo",
                        "tar-zstd-v1",
                        CacheKeyInputs::default(),
                        CacheKeyInputs::default(),
                    )
                    .unwrap(),
                ],
                Default::default(),
                vec![WorkflowStep::new("Test", "true").unwrap()],
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let revision = WorkflowRevision::new(workflow.clone(), definition).unwrap();
    let source_digest = "c".repeat(64);
    let source = RunSource::ephemeral_git_bundle(scope_domain::content::SourceBlob {
        content_ref: ContentRef::git_bundle_sha256(source_digest.clone()),
        sha256: source_digest,
        git_oid: "d".repeat(40),
        git_file_mode: "100644".to_string(),
        size_bytes: 1,
    })
    .unwrap();
    let runs = fixture.state.metadata.runs();
    runs.enqueue_run(
        Run::new(
            "run-cache-run",
            "run-cache-test",
            workflow,
            revision.digest(),
            RunTrigger::Manual,
            Some("user_run_cache".to_string()),
            source,
            fixture.now,
        )
        .unwrap(),
        revision,
    )
    .await
    .unwrap();
    runs.dispatch_job(
        "run-cache-run",
        "checks",
        ATTEMPT_ID,
        &machine_token_hash(ATTEMPT_TOKEN),
        "test-runtime",
        fixture.now,
        fixture.now + 3_600,
    )
    .await
    .unwrap();
}

async fn post_json(
    app: &axum::Router,
    path: &str,
    body: &impl serde::Serialize,
) -> (StatusCode, Vec<u8>) {
    let response = app
        .clone()
        .oneshot(
            Request::post(path)
                .header(AUTHORIZATION, format!("Bearer {ATTEMPT_TOKEN}"))
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn attempt_cache_round_trip_is_cold_then_warm_then_unchanged_and_ends_with_the_attempt() {
    let fixture = Fixture::new().await;
    dispatch_attempt(&fixture).await;
    let app = router(fixture.state.clone());
    let cache = AttemptCacheKey {
        cache_name: "cargo".to_string(),
        compatibility_inputs_digest: "1".repeat(64),
        exact_inputs_digest: "2".repeat(64),
    };
    let restore_path = routes::attempt_cache_restore(ATTEMPT_ID);
    let restore = RestoreAttemptCacheRequest {
        cache: cache.clone(),
    };
    let (status, cold) = post_json(&app, &restore_path, &restore).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<RestoreAttemptCacheResponse>(&cold).unwrap(),
        RestoreAttemptCacheResponse::Miss
    );

    let undeclared = RestoreAttemptCacheRequest {
        cache: AttemptCacheKey {
            cache_name: "npm".to_string(),
            ..cache.clone()
        },
    };
    assert_eq!(
        post_json(&app, &restore_path, &undeclared).await.0,
        StatusCode::BAD_REQUEST
    );

    let object_bytes = b"run cache round trip".to_vec();
    let object_digest = hex::encode(Sha256::digest(&object_bytes));
    let prepare_path = routes::attempt_cache_upload_prepare(ATTEMPT_ID);
    let prepare = PrepareAttemptCacheUploadRequest {
        cache: cache.clone(),
        object_digest: object_digest.clone(),
        size_bytes: object_bytes.len() as u64,
    };
    let (status, prepared) = post_json(&app, &prepare_path, &prepare).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&prepared)
    );
    let PrepareAttemptCacheUploadResponse::Upload {
        lease_id,
        upload_url,
        upload_headers,
        ..
    } = serde_json::from_slice(&prepared).unwrap()
    else {
        panic!("first prepare must upload");
    };
    let mut put = reqwest::Client::new()
        .put(upload_url)
        .body(object_bytes.clone());
    for (name, value) in upload_headers {
        put = put.header(name, value);
    }
    assert!(put.send().await.unwrap().status().is_success());

    let commit_path = routes::attempt_cache_upload_commit(ATTEMPT_ID);
    let commit = CommitAttemptCacheUploadRequest {
        cache: cache.clone(),
        lease_id,
        object_digest,
        size_bytes: object_bytes.len() as u64,
    };
    let other_cache_commit = CommitAttemptCacheUploadRequest {
        cache: AttemptCacheKey {
            exact_inputs_digest: "3".repeat(64),
            ..cache.clone()
        },
        ..commit.clone()
    };
    assert_eq!(
        post_json(&app, &commit_path, &other_cache_commit).await.0,
        StatusCode::FORBIDDEN
    );
    for _ in 0..2 {
        assert_eq!(
            post_json(&app, &commit_path, &commit).await.0,
            StatusCode::NO_CONTENT
        );
    }

    let (status, warm) = post_json(&app, &restore_path, &restore).await;
    assert_eq!(status, StatusCode::OK);
    let RestoreAttemptCacheResponse::Hit { download_url, .. } =
        serde_json::from_slice(&warm).unwrap()
    else {
        panic!("committed cache must restore");
    };
    assert_eq!(
        reqwest::get(download_url)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        object_bytes
    );
    let (_, unchanged) = post_json(&app, &prepare_path, &prepare).await;
    assert!(matches!(
        serde_json::from_slice(&unchanged).unwrap(),
        PrepareAttemptCacheUploadResponse::UseObject { .. }
    ));

    fixture
        .state
        .metadata
        .runs()
        .abandon_attempt(
            ATTEMPT_ID,
            &machine_token_hash(ATTEMPT_TOKEN),
            unix_now().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        post_json(&app, &restore_path, &restore).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn expired_uploads_drain_multiple_batches_with_bounded_deletions() {
    let fixture = Fixture::new().await;
    for index in 1..=201 {
        fixture.upload(index, fixture.now - 3600, false).await;
    }
    reconcile_at(&fixture.state, fixture.now).await.unwrap();
    assert_eq!(fixture.deletes(), 201);
    assert!(
        (2..=DELETE_CONCURRENCY).contains(&fixture.objects.peak_deletes.load(Ordering::SeqCst))
    );
    reconcile_at(&fixture.state, fixture.now).await.unwrap();
    assert_eq!(fixture.deletes(), 201);
}

#[tokio::test]
async fn failed_uploads_wait_for_claim_expiry_before_retrying() {
    let fixture = Fixture::new().await;
    for index in 1..=101 {
        fixture.upload(index, fixture.now - 3600, false).await;
    }
    fixture.objects.fail_deletes.store(true, Ordering::SeqCst);
    reconcile_at(&fixture.state, fixture.now).await.unwrap();
    assert_eq!(fixture.deletes(), 101);
    fixture.objects.fail_deletes.store(false, Ordering::SeqCst);
    reconcile_at(&fixture.state, fixture.now + RETRY_SECONDS - 1)
        .await
        .unwrap();
    assert_eq!(fixture.deletes(), 101);
    reconcile_at(&fixture.state, fixture.now + RETRY_SECONDS)
        .await
        .unwrap();
    assert_eq!(fixture.deletes(), 202);
    reconcile_at(&fixture.state, fixture.now + RETRY_SECONDS * 2)
        .await
        .unwrap();
    assert_eq!(fixture.deletes(), 202);
}

#[tokio::test]
async fn object_deletions_drain_batches_without_shortening_reference_grace() {
    let fixture = Fixture::new().await;
    let old = fixture.now - 8 * 24 * 60 * 60;
    for index in 1..=201 {
        fixture.upload(index, old, true).await;
    }
    let caches = fixture.state.metadata.caches();
    assert_eq!(
        caches
            .expire_references(fixture.now - 3601, 1000)
            .await
            .unwrap(),
        201
    );
    fixture.upload(202, old, true).await;
    reconcile_at(&fixture.state, fixture.now).await.unwrap();
    assert_eq!(fixture.deletes(), 201);
    assert_eq!(
        caches
            .expire_committed_uploads(fixture.now, 1000)
            .await
            .unwrap(),
        0
    );
    let after_sweep = unix_now().unwrap();
    assert_eq!(
        caches
            .claim_deletions(after_sweep + 3601, after_sweep + 7200, 1000)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn hung_upload_delete_is_bounded_and_retried_after_claim_expiry() {
    let fixture = Fixture::new().await;
    fixture.upload(1, fixture.now - 3600, false).await;
    fixture.objects.hang_deletes.store(true, Ordering::SeqCst);
    tokio::time::timeout(
        Duration::from_secs(3),
        reconcile_at(&fixture.state, fixture.now),
    )
    .await
    .expect("hung upload cleanup must release its transaction within the timeout")
    .unwrap();
    assert_eq!(fixture.deletes(), 1);
    fixture.objects.hang_deletes.store(false, Ordering::SeqCst);
    tokio::time::timeout(
        Duration::from_secs(3),
        reconcile_at(&fixture.state, fixture.now + RETRY_SECONDS),
    )
    .await
    .expect("the next pass must reclaim the row lock and retry")
    .unwrap();
    assert_eq!(fixture.deletes(), 2);
    reconcile_at(&fixture.state, fixture.now + RETRY_SECONDS * 2)
        .await
        .unwrap();
    assert_eq!(fixture.deletes(), 2);
}
