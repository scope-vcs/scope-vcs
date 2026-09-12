mod http_surface;
use crate::{
    app::router,
    auth::{clerk::*, tokens::*},
    config::*,
    git::{command::*, import::*, projection_repo::*, storage::*, upload::*, *},
    http::responses::*,
    push_intents::*,
    repo_access::*,
    runtime_budgets::{BudgetedObjectStore, RuntimeBudgetConfig, RuntimeBudgets},
    state::*,
    use_cases::{
        content_cleanup::*,
        git_receive::{self as git_receive_use_case, ReceivePackAccess},
    },
};
use axum::{
    body::{Body, to_bytes},
    http::{
        HeaderMap, Request, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE},
    },
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, jwk::JwkSet};
use scope_domain::policy::{Policy, ScopePath, Visibility, VisibilityRule};
use scope_domain::projection::{
    FileChange, LogicalCommit, ProjectionViewKey, SourceGraph, project_graph,
};
use scope_domain::{
    account::UserAccount,
    projection::LogicalCommitOrigin,
    repo_actions::RepoStorageCleanup,
    repo_config::{ConfigVisibility, RepoConfig},
    repository::collaboration::{
        RepositoryInvite, RepositoryInviteState, RepositoryMember, RepositoryMemberPermissions,
    },
    repository::credentials::GitPushToken,
    repository::{RepoLifecycleState, RepoRecord, Repository},
};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_object_store::{
    ContentObjectKind, MemoryObjectStore, put_source_blob, source_blob_bytes,
};
use std::{
    collections::BTreeMap,
    fs,
    ops::Deref,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;

mod admin;
mod auth;
mod auth_fixtures;
mod cli_auth;
mod clone_access;
mod cloud_runs;
mod dependencies;
mod device_login;
mod git_binary;
mod git_http;
mod git_http_gzip;
mod git_import_validation;
mod git_projection_identity;
mod git_receive;
mod git_receive_config;
mod git_request_refs;
mod history;
mod http;
mod landing_file;
mod manual_runs;
mod push_intent_completion;
mod repo_cleanup;
mod repo_events;
mod repo_lifecycle;
mod repo_metadata;
mod repo_visibility;
mod request_attachments;
mod request_attention;
mod request_discussions;
mod requests;
mod run_inspection;
mod run_resources;
mod runtime_budgets;

use auth_fixtures::*;
use http::api_request;

const TEST_REPO_OWNER: &str = "owner";
const TEST_REPO_NAME: &str = "repo";
const TEST_REPO_ID: &str = "owner/repo";

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn test_owner_id() -> String {
    scope_postgres::db::scope_user_id_for_auth_identity("clerk", TEST_CLERK_USER_ID)
}

fn test_user(id: impl Into<String>, handle: &str, email: &str) -> UserAccount {
    UserAccount {
        id: id.into(),
        handle: handle.to_string(),
        email: email.to_string(),
        email_verified: true,
    }
}

fn test_state_with_repo() -> AppState {
    let owner_id = test_owner_id();
    let owner = test_user(&owner_id, TEST_REPO_OWNER, TEST_OWNER_EMAIL);
    let repo = test_repo(&owner_id);
    let state = AppState::test_state();
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(scope_postgres::db::CatalogFixture {
            users: BTreeMap::from([(owner.id.clone(), owner)]),
            repositories: BTreeMap::from([(repo.record.id.clone(), repo)]),
            ..Default::default()
        })
        .unwrap();
    state
}

async fn replace_test_repo(state: &AppState, repo: Repository) {
    state
        .metadata
        .repositories()
        .replace_repository_for_tests(repo)
        .await
        .unwrap();
}

async fn test_state_with_readme() -> AppState {
    let state = test_state_with_repo();
    replace_test_repo(&state, repo_with_readme(&state)).await;
    state
}

async fn test_state_with_git_push_token(secret: &str) -> AppState {
    let state = test_state_with_repo();
    let mut repo = repo_with_readme(&state);
    repo.git_push_token = Some(GitPushToken {
        token_hash: token_hash(secret),
        owner_user_id: repo.record.owner_user_id.clone(),
        created_at_unix: unix_now(),
    });
    replace_test_repo(&state, repo).await;
    state
}

async fn test_state_with_first_push_token() -> (AppState, String) {
    let state = test_state_with_repo();
    let (secret, token) = generate_first_push_token(&test_owner_id()).unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.record.lifecycle_state = RepoLifecycleState::AwaitingFirstPush;
            repo.first_push_token = Some(token);
        })
        .await
        .unwrap();
    (state, secret)
}

fn test_state_with_jwks() -> AppState {
    let state = AppState::test_state();
    cache_test_jwks(&state);
    state
}

async fn response_json(response: Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn assert_text_content(value: &serde_json::Value, expected: &str) {
    assert_eq!(value["kind"], "text");
    assert_eq!(value["text"], expected);
}

struct TempGitRepo(PathBuf);

impl Deref for TempGitRepo {
    type Target = FsPath;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<FsPath> for TempGitRepo {
    fn as_ref(&self) -> &FsPath {
        &self.0
    }
}

impl Drop for TempGitRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_git_repo(label: &str) -> TempGitRepo {
    let repo = unique_test_path(label);
    let _ = fs::remove_dir_all(&repo);
    fs::create_dir_all(&repo).unwrap();
    run_git(
        None,
        &["init", "-b", "main", repo.to_str().unwrap()],
        "init test repo",
    )
    .unwrap();
    fs::create_dir_all(repo.join(".scope")).unwrap();
    fs::write(repo.join(".scope/RULES.md"), []).unwrap();
    run_git(
        Some(&repo),
        &["add", ".scope/RULES.md"],
        "stage canonical repo rules",
    )
    .unwrap();
    TempGitRepo(repo)
}

fn unique_test_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "scope-vcs-{label}-{}-{}",
        std::process::id(),
        unix_now()
    ))
}

fn clone_test_repo(source: &FsPath, label: &str, bare: bool) -> TempGitRepo {
    let repo = unique_test_path(label);
    let _ = fs::remove_dir_all(&repo);
    let mut args = vec!["clone"];
    if bare {
        args.push("--bare");
    }
    args.extend([source.to_str().unwrap(), repo.to_str().unwrap()]);
    run_git(None, &args, "clone test repo").unwrap();
    TempGitRepo(repo)
}

fn commit_all(repo: &FsPath, message: &str) {
    run_git(
        Some(repo),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "commit",
            "-m",
            message,
        ],
        "commit test repo",
    )
    .unwrap();
}

fn clone_with_bearer(remote: &str, destination: &FsPath, bearer_header_value: &str, action: &str) {
    let header = format!("http.{remote}.extraHeader=Authorization: {bearer_header_value}");
    run_git(
        None,
        &[
            "-c",
            &header,
            "clone",
            remote,
            destination.to_str().unwrap(),
        ],
        action,
    )
    .unwrap();
}

const TEST_PUSH_HEAD_OID: &str = "1111111111111111111111111111111111111111";

async fn insert_push_intent_header(
    state: &AppState,
    headers: &mut HeaderMap,
    user_id: &str,
    head_oid: &str,
) {
    let token = create_test_push_intent(state, user_id, head_oid).await;
    headers.insert("x-scope-push-intent", token.parse().unwrap());
}

async fn configure_push_intent_header(
    state: &AppState,
    repo: &FsPath,
    remote: &str,
    user_id: &str,
) {
    let head_oid = git_head_oid(repo);
    let token = create_test_push_intent(state, user_id, &head_oid).await;
    let key = format!("http.{remote}.extraHeader");
    run_git(
        Some(repo),
        &[
            "config",
            "--add",
            key.as_str(),
            &format!("X-Scope-Push-Intent: {token}"),
        ],
        "configure push intent header",
    )
    .unwrap();
}

async fn create_test_push_intent(state: &AppState, user_id: &str, head_oid: &str) -> String {
    let repo = find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let config = repo.repo_config.clone();
    state
        .create_push_intent(
            TEST_REPO_ID,
            user_id,
            head_oid,
            config.clone(),
            repo_config_fingerprint(&config).unwrap(),
            repo.git_head.as_ref().map(|head| head.frontier()),
        )
        .unwrap()
        .token
}

fn authorization_headers(value: impl AsRef<str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, value.as_ref().parse().unwrap());
    headers
}

fn git_push_token_headers(secret: &str) -> HeaderMap {
    authorization_headers(format!(
        "Basic {}",
        BASE64.encode(format!("scope:{secret}"))
    ))
}

struct TestServer(tokio::task::JoinHandle<()>);

impl Drop for TestServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn spawn_test_server(state: &AppState) -> (String, TestServer) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let state = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    (origin, TestServer(server))
}

async fn live_file_content(state: &AppState, path: &str) -> Option<String> {
    let repo = find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    match repo.live_files.get(&ScopePath::parse(path).unwrap()) {
        Some(blob) => Some(blob_content(state, blob, &repo).await),
        None => None,
    }
}

async fn persist_test_update(
    state: &AppState,
    update: impl Into<TestReceivePackUpdate>,
) -> Result<scope_domain::repository::git::GitHead, crate::error::ApiError> {
    persist_and_promote_test_update(state, update, &test_owner_id()).await
}

fn test_git_segment_ref(label: &str) -> scope_domain::repository::git::GitSegmentRef {
    use sha2::{Digest, Sha256};

    scope_domain::repository::git::GitSegmentRef {
        segment_id: format!("test-{}", hex::encode(Sha256::digest(label.as_bytes()))),
        sha256: hex::encode(Sha256::digest(format!("pack:{label}").as_bytes())),
        plaintext_bytes: label.len() as u64,
        encoding_version: scope_git_storage::ENCODING_VERSION,
    }
}

async fn ready_test_git_segment(
    state: &AppState,
    label: &str,
) -> scope_domain::repository::git::GitSegmentRef {
    let reservation = state.git_segment_store.reserve(TEST_REPO_ID).unwrap();
    state
        .metadata
        .repositories()
        .begin_git_segment_upload(
            TEST_REPO_ID,
            &reservation.segment_id,
            &reservation.object_key,
            scope_git_storage::ENCODING_VERSION,
            crate::persistence::unix_now().unwrap(),
        )
        .await
        .unwrap();
    let staged = state
        .git_segment_store
        .ingest_reserved_blocking_reader(
            TEST_REPO_ID,
            reservation,
            std::io::Cursor::new(format!("test segment {label}").into_bytes()),
            u64::MAX,
            None,
        )
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .mark_git_segment_upload_ready(
            &staged.segment,
            staged.encrypted_bytes,
            crate::persistence::unix_now().unwrap(),
        )
        .await
        .unwrap();
    state.git_segment_store.delete_local(&staged).await.unwrap();
    staged.segment
}

enum TestReceivePackUpdate {
    Prepared(Box<PreparedReceivePackUpdate>),
    Raw(Box<ReceivePackUpdate>),
}

impl From<PreparedReceivePackUpdate> for TestReceivePackUpdate {
    fn from(update: PreparedReceivePackUpdate) -> Self {
        Self::Prepared(Box::new(update))
    }
}

impl From<ReceivePackUpdate> for TestReceivePackUpdate {
    fn from(update: ReceivePackUpdate) -> Self {
        Self::Raw(Box::new(update))
    }
}

async fn persist_and_promote_test_update(
    state: &AppState,
    update: impl Into<TestReceivePackUpdate>,
    actor_id: &str,
) -> Result<scope_domain::repository::git::GitHead, crate::error::ApiError> {
    let prepared = match update.into() {
        TestReceivePackUpdate::Prepared(prepared) => *prepared,
        TestReceivePackUpdate::Raw(update) => {
            let mut update = *update;
            let reservation = state.git_segment_store.reserve(TEST_REPO_ID).unwrap();
            state
                .metadata
                .repositories()
                .begin_git_segment_upload(
                    TEST_REPO_ID,
                    &reservation.segment_id,
                    &reservation.object_key,
                    scope_git_storage::ENCODING_VERSION,
                    crate::persistence::unix_now()?,
                )
                .await?;
            let staged_segment = state
                .git_segment_store
                .ingest_reserved_blocking_reader(
                    TEST_REPO_ID,
                    reservation,
                    std::io::Cursor::new(b"test Git pack segment".to_vec()),
                    u64::MAX,
                    None,
                )
                .await
                .map_err(|error| crate::error::ApiError::internal_message(error.to_string()))?;
            state
                .metadata
                .repositories()
                .mark_git_segment_upload_ready(
                    &staged_segment.segment,
                    staged_segment.encrypted_bytes,
                    crate::persistence::unix_now()?,
                )
                .await?;
            update.git_pack_span.segment = staged_segment.segment.clone();
            let write_lease = state
                .metadata
                .repositories()
                .acquire_git_write_lease(TEST_REPO_ID)
                .await?;
            let upload_heartbeat = crate::git::import::GitSegmentUploadHeartbeat::start(
                state,
                staged_segment.segment.segment_id.clone(),
            );
            PreparedReceivePackUpdate {
                update,
                staged_segment,
                write_lease,
                upload_heartbeat,
            }
        }
    };
    let persisted = git_receive_use_case::main_push::persist_main_push(
        state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        prepared,
        actor_id,
        &test_repo_incarnation(),
    )
    .await?;
    let head = persisted.head;
    state
        .git_segment_store
        .delete_local(&persisted.staged_segment)
        .await
        .map_err(|error| crate::error::ApiError::internal_message(error.to_string()))?;
    persisted.write_lease.release().await;
    Ok(head)
}

async fn receive_pack_access(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
) -> Result<ReceivePackAccess, crate::error::ApiError> {
    let (authorization, push_intent) = crate::git::receive_pack_credentials(state, headers).await?;
    git_receive_use_case::authorize(
        state,
        owner,
        repo_name,
        authorization,
        push_intent.as_deref(),
    )
    .await
}

async fn published_staging_repo(state: &AppState) -> PathBuf {
    ensure_ready_receive_pack_staging_repo(
        state,
        &test_repo_incarnation(),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &test_owner_id(),
    )
    .await
    .unwrap()
}

fn test_repo_incarnation() -> scope_domain::repository::RepositoryIncarnation {
    scope_domain::repository::RepositoryIncarnation::new(TEST_REPO_ID, "repoi_workflow_test")
        .expect("test repository identity is valid")
}

fn git_head_oid(repo: &FsPath) -> String {
    git_stdout_text(repo, &["rev-parse", "HEAD"], "read git head")
        .unwrap()
        .trim()
        .to_string()
}

fn test_repo(owner_id: &str) -> Repository {
    Repository {
        record: RepoRecord {
            id: TEST_REPO_ID.to_string(),
            incarnation_id: "repoi_workflow_test".to_string(),
            owner_handle: TEST_REPO_OWNER.to_string(),
            name: TEST_REPO_NAME.to_string(),
            owner_user_id: owner_id.to_string(),
            description: None,
            website_url: None,
            lifecycle_state: RepoLifecycleState::Ready,
            change_version: 1,
        },
        repo_config: RepoConfig::with_default_visibility(ConfigVisibility::Public),
        first_push_token: None,
        git_push_token: None,
        policy: Policy::new(Visibility::Public),
        graph: SourceGraph {
            repo_id: TEST_REPO_ID.to_string(),
            commits: Vec::new(),
        },
        visibility_change_sets: Vec::new(),
        live_files: BTreeMap::new(),
        git_head: None,
        git_pack_spans: Vec::new(),
        members: Vec::new(),
        invitations: Vec::new(),
    }
}

fn test_repository_member(
    repo_id: impl Into<String>,
    user_id: impl Into<String>,
    permissions: RepositoryMemberPermissions,
) -> RepositoryMember {
    RepositoryMember {
        repo_id: repo_id.into(),
        user_id: user_id.into(),
        permissions,
        created_at_unix: 10,
        updated_at_unix: 10,
    }
}

fn member_permissions(
    can_push: bool,
    can_change_file_visibility: bool,
    can_apply_changes: bool,
) -> RepositoryMemberPermissions {
    RepositoryMemberPermissions {
        can_push,
        can_change_file_visibility,
        can_apply_changes,
    }
}

async fn apply_first_push_from_staging_repo(
    state: &AppState,
    staging_repo: &FsPath,
    config: RepoConfig,
) {
    let update = reviewed_update_from_staging_repo(
        state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        staging_repo,
        &test_owner_id(),
        config,
        ReviewedUpdateMode::FirstPush,
    )
    .await
    .unwrap();
    persist_test_update(state, update).await.unwrap();
}

fn source_blob(state: &AppState, content: &str) -> scope_domain::content::SourceBlob {
    put_source_blob(state.object_store.as_ref(), content.as_bytes()).unwrap()
}

async fn blob_content(
    state: &AppState,
    blob: &scope_domain::content::SourceBlob,
    repo: &Repository,
) -> String {
    let git_source = repo
        .git_head
        .as_ref()
        .map(|head| (repo.incarnation(), head, repo.git_pack_spans.as_slice()));
    String::from_utf8(
        crate::git::content::source_content_bytes(state, blob, git_source)
            .await
            .unwrap(),
    )
    .unwrap()
}

fn repo_with_readme(state: &AppState) -> Repository {
    let mut repo = test_repo(&test_owner_id());
    let path = ScopePath::parse("/README.md").unwrap();
    let content = source_blob(state, "hello");
    let rules_path = ScopePath::parse("/.scope/RULES.md").unwrap();
    let rules_content = source_blob(state, "");
    repo.graph.commits.push(LogicalCommit {
        occurred_at_unix: None,
        id: "rv1".to_string(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: "rv1".to_string(),
        },
        author_id: repo.record.owner_user_id.clone(),
        message: "initial".to_string(),
        changes: vec![
            FileChange {
                visibility: Visibility::Public,
                path: path.clone(),
                old_content: None,
                new_content: Some(content.clone()),
            },
            FileChange {
                visibility: Visibility::Public,
                path: rules_path.clone(),
                old_content: None,
                new_content: Some(rules_content.clone()),
            },
        ],
    });
    repo.live_files.insert(path, content);
    repo.live_files.insert(rules_path, rules_content);
    repo
}

fn receive_pack_update(state: &AppState, changes: Vec<(&str, Option<&str>)>) -> ReceivePackUpdate {
    let config = repo_config(Visibility::Public);
    let head_oid = "1111111111111111111111111111111111111111";
    ReceivePackUpdate {
        occurred_at_unix: None,
        branch: format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        head_oid: head_oid.to_string(),
        base_git_frontier: None,
        author_id: test_owner_id(),
        message: "owner push".to_string(),
        git_head: scope_domain::repository::git::GitHead::new(
            "1111111111111111111111111111111111111111".to_string(),
            1,
            1,
        ),
        git_pack_span: scope_domain::repository::git::GitPackSpan {
            first_sequence: 1,
            last_sequence: 1,
            geometric_tier: 0,
            base_oid: None,
            head_oid: "1111111111111111111111111111111111111111".to_string(),
            segment: test_git_segment_ref("test staged Git segment"),
        },
        workflow_catalog: scope_domain::runs::catalog::RepositoryWorkflowCatalog::captured(
            TEST_REPO_ID,
            head_oid,
            2,
            Vec::new(),
        )
        .unwrap(),
        landing_file_mutation: scope_domain::landing_file::RepositoryLandingFileMutation::Unchanged,
        previous_config: None,
        base_config_hash: repo_config_fingerprint(&config).unwrap(),
        config,
        changes: changes
            .into_iter()
            .map(|(path, content)| ReceivePackFileChange {
                path: repo_scope_path(path).unwrap(),
                content: content.map(|content| source_blob(state, content)),
            })
            .collect(),
    }
}

fn repo_config(default_visibility: Visibility) -> RepoConfig {
    RepoConfig::with_default_visibility(default_visibility.into())
}

fn push_intent_request_json(head_oid: &str, config: RepoConfig) -> String {
    push_intent_request_json_with_base(
        head_oid,
        repo_config_fingerprint(&repo_config(Visibility::Public)).unwrap(),
        config,
    )
}

fn push_intent_request_json_with_base(
    head_oid: &str,
    base_config_hash: String,
    config: RepoConfig,
) -> String {
    serde_json::json!({
        "head_oid": head_oid,
        "base_config_hash": base_config_hash,
        "config": config,
    })
    .to_string()
}

struct DeleteFailsObjectStore;

impl scope_object_store::ObjectStore for DeleteFailsObjectStore {
    fn put(&self, _key: &str, _bytes: Vec<u8>) -> Result<(), scope_object_store::ObjectStoreError> {
        Ok(())
    }

    fn get(&self, _key: &str) -> Result<Vec<u8>, scope_object_store::ObjectStoreError> {
        Err(scope_object_store::ObjectStoreError::not_found(
            "object not found",
        ))
    }

    fn delete(&self, _key: &str) -> Result<(), scope_object_store::ObjectStoreError> {
        Err(scope_object_store::ObjectStoreError::service_unavailable(
            "delete failed",
        ))
    }
}

struct PutFailsObjectStore {
    readable: Arc<MemoryObjectStore>,
}

impl scope_object_store::ObjectStore for PutFailsObjectStore {
    fn put(&self, _key: &str, _bytes: Vec<u8>) -> Result<(), scope_object_store::ObjectStoreError> {
        Err(scope_object_store::ObjectStoreError::service_unavailable(
            "object PUT failed for test",
        ))
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, scope_object_store::ObjectStoreError> {
        scope_object_store::ObjectStore::get(self.readable.as_ref(), key)
    }

    fn delete(&self, key: &str) -> Result<(), scope_object_store::ObjectStoreError> {
        scope_object_store::ObjectStore::delete(self.readable.as_ref(), key)
    }
}

const WORKFLOW: &str = r#"
name: Test
on:
  manual: true
caches: []
container:
  image: alpine@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
timeout: 5m
jobs:
  checks:
    steps:
      - name: Test
        run: printf 'hello from runner\n'
"#;

fn workflow_named(name: &str) -> String {
    WORKFLOW.replacen("name: Test", &format!("name: {name}"), 1)
}

fn logical_commit(id: &str, message: &str, changes: Vec<FileChange>) -> LogicalCommit {
    LogicalCommit {
        occurred_at_unix: None,
        id: id.into(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: id.to_string(),
        },
        author_id: test_owner_id(),
        message: message.into(),
        changes,
    }
}

fn history_change(
    path: &str,
    visibility: Visibility,
    old: Option<scope_domain::content::SourceBlob>,
    new: Option<scope_domain::content::SourceBlob>,
) -> FileChange {
    FileChange {
        path: ScopePath::parse(path).unwrap(),
        visibility,
        old_content: old,
        new_content: new,
    }
}

async fn drain_outbox(state: &AppState, label: &str) -> scope_postgres::db::OutboxRunSummary {
    let report = state
        .metadata
        .jobs()
        .run_ready_outbox_jobs(
            label,
            10,
            &|| {
                crate::persistence::unix_now()
                    .map_err(crate::error::ApiError::into_operator_diagnostic)
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    assert_eq!(report.failed, 0);
    report
}
