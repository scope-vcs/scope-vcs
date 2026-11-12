use super::super::custom_views::{custom_view, member_id, view};
use super::super::*;
use super::Caller;
use super::seeds::{
    GITHUB_JOB_ID, GITHUB_JOB_NAME, GITHUB_RUN_ID, GITHUB_WORKFLOW_NAME, seed_github_run,
};
use super::write_bodies::{WriteBodies, write_bodies};
use scope_api_contract::routes;
use scope_domain::{
    runs::{
        log::RunLogChunk,
        run::Run,
        source::{RunSource, RunTrigger},
    },
    views::{ViewDefinition, Views},
};

pub(super) const OUTSIDER_SUBJECT: &str = "user_isolation_outsider";
pub(super) const OUTSIDER_EMAIL: &str = "isolation-outsider@example.com";
const OUTSIDER_HANDLE: &str = "isolation-outsider";
const UNHELD_CAPABILITY: &str = "unheld-capability";
pub(super) const VISIBLE_PATH: &str = "src/visible.rs";
const VISIBLE_CONTENT: &str = "narrow view reads visible marker";
const OPS_PATH: &str = "ops/hidden.txt";
const OPS_CONTENT: &str = "ops view only marker";
const PRIVATE_PATH: &str = "notes.txt";
const PRIVATE_CONTENT: &str = "private view only marker";
const REVISION_CONTENT: &str = "private request revision marker";
const COMMIT_MESSAGE: &str = "repo a commit message marker";
const REQUEST_NAME: &str = "repo-a-request-title-marker";
const REQUEST_DESCRIPTION: &str = "repo a request description marker";
const DISCUSSION_BODY: &str = "repo a discussion body marker";
const REPLY_BODY: &str = "repo a reply body marker";
const ATTACHMENT_FILENAME: &str = "repo-a-attachment-marker.jpg";
const INVITE_EMAIL: &str = "repo-a-invitee-marker@example.com";
const LOG_TEXT: &str = "repo a run log marker";
const REPO_DESCRIPTION: &str = "repo a description marker";

pub(super) struct RepoA {
    pub(super) app: axum::Router,
    pub(super) params: BTreeMap<&'static str, String>,
    pub(super) absent: BTreeMap<&'static str, String>,
    pub(super) children_in_b: BTreeMap<&'static str, String>,
    pub(super) absent_children_in_b: BTreeMap<&'static str, String>,
    pub(super) queries: Vec<String>,
    pub(super) write_query: String,
    pub(super) write_bodies: WriteBodies,
    pub(super) outsider_markers: Vec<String>,
    pub(super) narrow_markers: Vec<String>,
}

fn absent_twin(value: &str) -> String {
    let mut chars = value.chars().collect::<Vec<_>>();
    let last = chars
        .iter()
        .rposition(char::is_ascii_alphanumeric)
        .unwrap_or_else(|| panic!("{value} has no character to vary"));
    chars[last] = match chars[last] {
        '9' | 'z' => 'a',
        'f' => '0',
        'Z' => 'A',
        c => char::from(c as u8 + 1),
    };
    chars.into_iter().collect()
}

fn isolation_config() -> RepoConfig {
    let mut config = repo_config(ViewId::private());
    let mut definitions = Vec::<ViewDefinition>::from(Views::builtin());
    definitions.extend([
        custom_view("agent", "Agent", &[]),
        custom_view("ops", "Ops", &[]),
    ]);
    config.views = Views::new(definitions).unwrap();
    config.files.rules = [("/src/**", "agent"), ("/ops/**", "ops")]
        .into_iter()
        .map(
            |(path, label)| scope_domain::repo_config::RepoConfigFileRule {
                path: path.to_string(),
                view: view(label),
            },
        )
        .collect();
    config
}

fn outsider_repo() -> Repository {
    let outsider_id =
        scope_postgres::db::scope_user_id_for_auth_identity("clerk", OUTSIDER_SUBJECT);
    let mut repo = test_repo(&outsider_id);
    repo.record.id = format!("{OUTSIDER_HANDLE}/repo-b");
    repo.record.incarnation_id = "repoi_isolation_b".to_string();
    repo.record.owner_handle = OUTSIDER_HANDLE.to_string();
    repo.record.name = "repo-b".to_string();
    repo.graph.repo_id = repo.record.id.clone();
    repo
}

async fn owner_json(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<String>,
) -> serde_json::Value {
    expect_json(
        api_request(
            router(state.clone()),
            method,
            uri,
            Some(&bearer_header()),
            body.as_deref(),
        )
        .await,
        StatusCode::OK,
    )
    .await
}

struct PushedRepoA {
    head: String,
    source: TempGitRepo,
}

async fn push_repo_a(state: &AppState) -> PushedRepoA {
    let source = temp_git_repo("cross-repo-isolation");
    for (path, content) in [
        (VISIBLE_PATH, VISIBLE_CONTENT),
        (OPS_PATH, OPS_CONTENT),
        (PRIVATE_PATH, PRIVATE_CONTENT),
    ] {
        let path = source.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    run_git(Some(&source), &["add", "-A"], "stage repo A").unwrap();
    commit_all(&source, COMMIT_MESSAGE);
    let bare = clone_test_repo(&source, "cross-repo-isolation-bare", true);
    apply_first_push_from_staging_repo(state, &bare, isolation_config()).await;
    PushedRepoA {
        head: git_head_oid(&bare),
        source,
    }
}

async fn push_request_revision(state: &AppState, source: &FsPath) -> String {
    let (origin, _server) = spawn_test_server(state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(source, &remote, &bearer_header());
    run_git(
        Some(source),
        &["checkout", "-b", REQUEST_NAME],
        "branch the request",
    )
    .unwrap();
    commit_file(source, PRIVATE_PATH, REVISION_CONTENT);
    run_git(
        Some(source),
        &["push", &remote, &format!("HEAD:refs/heads/{REQUEST_NAME}")],
        "push the request revision",
    )
    .unwrap();
    git_head_oid(source)
}

async fn active_run(state: &AppState) -> (String, String) {
    allow_native_runs(state).await;
    let revision = scope_run_config::parse_workflow(
        "/.scope/runs/isolation.yml",
        workflow_named("Isolation").as_bytes(),
    )
    .unwrap()
    .into_revision(TEST_REPO_ID.to_string())
    .unwrap();
    let run_id = "run_isolation".to_string();
    let attempt_id = format!("attempt_{run_id}");
    let mut source =
        scope_storage::content_object_for_bytes(ContentObjectKind::GitBundle, b"isolation bundle");
    source.git_oid = "b".repeat(40);
    let run = Run::new(
        run_id.clone(),
        format!("manual:{run_id}"),
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some(test_owner_id()),
        RunSource::ephemeral_git_bundle(source).unwrap(),
        1,
    )
    .unwrap();
    let runs = state.metadata.runs();
    runs.enqueue_run(run, revision).await.unwrap();
    let (bootstrap, attempt_token) = ("c".repeat(64), "d".repeat(64));
    runs.dispatch_job(
        &run_id,
        "checks",
        &attempt_id,
        &bootstrap,
        "test-runtime",
        2,
        100,
    )
    .await
    .unwrap();
    runs.claim_runtime(&attempt_id, &bootstrap, &attempt_token, 3, 100)
        .await
        .unwrap();
    runs.start_attempt_step(&attempt_id, &attempt_token, 0, 4)
        .await
        .unwrap();
    let chunk = RunLogChunk::new(&attempt_id, 0, 1, LOG_TEXT.to_string(), 5).unwrap();
    runs.append_attempt_log(chunk, &attempt_token, 5)
        .await
        .unwrap();
    (run_id, attempt_id)
}

fn grant_agents(state: &AppState) {
    let resource = crate::http::origins::public_mcp_resource().unwrap();
    let clerk = scripted_clerk(state);
    let mut tokens = clerk.oauth_tokens.lock().unwrap();
    for caller in [Caller::Outsider, Caller::NarrowMember, Caller::Owner] {
        let (token, subject) = caller.agent().unwrap();
        tokens.insert(
            token.to_string(),
            crate::clerk_backend::ClerkOAuthTokenCheck::Active(
                crate::clerk_backend::ClerkOAuthAccessToken {
                    subject: subject.to_string(),
                    scopes: vec!["read".to_string()],
                    audiences: vec![resource.clone()],
                },
            ),
        );
    }
}

pub(super) async fn repo_a() -> RepoA {
    let mut state = AppState::test_state();
    cache_test_jwks(&state);
    grant_agents(&state);
    let owner_id = test_owner_id();
    let outsider = outsider_repo();
    let mut repo_a_seed = test_repo(&owner_id);
    repo_a_seed.record.description = Some(REPO_DESCRIPTION.to_string());
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(scope_postgres::db::CatalogFixture {
            users: BTreeMap::from([
                (
                    owner_id.clone(),
                    test_user(&owner_id, TEST_REPO_OWNER, TEST_OWNER_EMAIL),
                ),
                (
                    outsider.record.owner_user_id.clone(),
                    test_user(
                        &outsider.record.owner_user_id,
                        OUTSIDER_HANDLE,
                        OUTSIDER_EMAIL,
                    ),
                ),
                (
                    member_id(),
                    test_user(
                        member_id(),
                        "isolation-member",
                        "isolation-member@example.com",
                    ),
                ),
            ]),
            repositories: BTreeMap::from([
                (TEST_REPO_ID.to_string(), repo_a_seed),
                (outsider.record.id.clone(), outsider),
            ]),
            ..Default::default()
        })
        .unwrap();
    super::super::fake_github::FakeGitHub::install(&mut state).await;
    let pushed = push_repo_a(&state).await;
    let head = pushed.head.clone();
    super::super::fake_github::connect_test_repository(
        &state,
        super::super::fake_github::GITHUB_REPOSITORY_ID,
        super::super::fake_github::GITHUB_FULL_NAME,
        0,
    )
    .await;
    seed_github_run(&state).await;
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.collaboration.members.push(test_repository_member(
                TEST_REPO_ID,
                member_id(),
                RepositoryMemberPermissions {
                    view: view("agent"),
                    ..Default::default()
                },
            ));
        })
        .await
        .unwrap();
    drain_outbox(&state, "cross-repo-isolation").await;

    let request_id =
        super::super::requests::start_private_request(&state, REQUEST_NAME, None).await;
    let request = format!("/v1/repos/{TEST_REPO_ID}/requests/{request_id}");
    owner_json(
        &state,
        "PATCH",
        &request,
        Some(serde_json::json!({ "description_markdown": REQUEST_DESCRIPTION }).to_string()),
    )
    .await;
    let revision_head = push_request_revision(&state, &pushed.source).await;
    let revisions = owner_json(&state, "GET", &format!("{request}/changes"), None).await;
    let revision_id = revisions["revisions"][0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no revision: {revisions}"))
        .to_string();
    let discussion = owner_json(
        &state,
        "POST",
        &format!("{request}/timeline"),
        Some(serde_json::json!({ "body_markdown": DISCUSSION_BODY, "client_discussion_id": "isolation" }).to_string()),
    )
    .await;
    let discussion_id = discussion["discussion"]["id"].as_str().unwrap().to_string();
    owner_json(
        &state,
        "POST",
        &format!("{request}/threads/{discussion_id}/replies"),
        Some(
            serde_json::json!({
                "body_markdown": REPLY_BODY,
                "client_reply_id": "isolation-reply",
                "reply_to_reply_id": null,
                "wait_after_reply": false,
            })
            .to_string(),
        ),
    )
    .await;
    let attachment = owner_json(
        &state,
        "POST",
        &format!("{request}/attachments/prepare"),
        Some(
            serde_json::json!({
                "operation_id": "isolation-attachment",
                "target": { "kind": "Description" },
                "filename": ATTACHMENT_FILENAME,
                "declared_media_type": "image/jpeg",
                "size_bytes": 4,
                "sha256": "a".repeat(64),
            })
            .to_string(),
        ),
    )
    .await;
    let invite = owner_json(
        &state,
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/invites"),
        Some(
            serde_json::json!({
                "email": INVITE_EMAIL,
                "permissions": RepositoryMemberPermissions::default(),
            })
            .to_string(),
        ),
    )
    .await;
    let invite_id = invite["id"].as_str().unwrap().to_string();
    owner_json(
        &state,
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/invites/{invite_id}/links"),
        None,
    )
    .await;
    let grant = owner_json(&state, "POST", routes::CLI_EXCHANGE_GRANTS, None).await;
    expect_json(
        api_request(
            router(state.clone()),
            "POST",
            routes::CLI_EXCHANGE_GRANTS_EXCHANGE,
            None,
            Some(&serde_json::json!({ "exchange_token": grant["exchange_token"] }).to_string()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let sessions = owner_json(&state, "GET", routes::CLI_SESSIONS, None).await;
    let (run_id, attempt_id) = active_run(&state).await;
    let history = owner_json(
        &state,
        "GET",
        &format!("/v1/repos/{TEST_REPO_ID}/history"),
        None,
    )
    .await;
    let entry_id = history["entries"][0]["source_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no history entry: {history}"))
        .to_string();

    let params = BTreeMap::from([
        ("owner", TEST_REPO_OWNER.to_string()),
        ("org", TEST_REPO_OWNER.to_string()),
        ("handle", TEST_REPO_OWNER.to_string()),
        ("repo", TEST_REPO_NAME.to_string()),
        ("view", "private".to_string()),
        ("request_id", request_id.clone()),
        ("revision_id", revision_id),
        ("commit_oid", revision_head),
        ("head_oid", head.clone()),
        ("discussion_id", discussion_id),
        (
            "attachment_id",
            attachment["attachment"]["id"].as_str().unwrap().to_string(),
        ),
        (
            "upload_id",
            attachment["transfer"]["upload_id"]
                .as_str()
                .unwrap()
                .to_string(),
        ),
        ("derivative_id", "preview".to_string()),
        ("part_number", "1".to_string()),
        ("invite_id", invite_id),
        ("member_user_id", member_id()),
        (
            "session_id",
            sessions["sessions"][0]["id"].as_str().unwrap().to_string(),
        ),
        ("run_id", run_id),
        ("attempt_id", attempt_id),
        ("step_index", "0".to_string()),
        ("job_id", "1".to_string()),
        ("github_run_id", GITHUB_RUN_ID.to_string()),
        ("github_job_id", GITHUB_JOB_ID.to_string()),
        ("entry_id", entry_id.clone()),
        ("token", UNHELD_CAPABILITY.to_string()),
        ("user_code", UNHELD_CAPABILITY.to_string()),
        ("device_code", UNHELD_CAPABILITY.to_string()),
    ]);
    let unvaried = [
        "view",
        "derivative_id",
        "part_number",
        "step_index",
        "job_id",
        "token",
        "user_code",
        "device_code",
    ];
    let absent: BTreeMap<_, _> = params
        .iter()
        .map(|(name, value)| {
            let value = if unvaried.contains(name) {
                value.clone()
            } else {
                absent_twin(value)
            };
            (*name, value)
        })
        .collect();
    let in_b = |values: &BTreeMap<&'static str, String>| {
        let mut values = values.clone();
        values.insert("owner", OUTSIDER_HANDLE.to_string());
        values.insert("repo", "repo-b".to_string());
        values
    };
    let children_in_b = in_b(&params);
    let absent_children_in_b = in_b(&absent);
    let common = format!(
        "section=active&after=0&workflow=test&git_oid={head}&commit_oid={head}&search=hidden&service=git-upload-pack"
    );
    let mut queries = vec![String::new()];
    for path in [OPS_PATH, PRIVATE_PATH] {
        queries.extend([
            format!("path={path}&view=private&{common}"),
            format!("path={path}&{common}"),
        ]);
    }
    queries.extend([
        format!("path={PRIVATE_PATH}&request={request_id}&{common}"),
        format!("path={OPS_PATH}&update={entry_id}&{common}"),
    ]);
    let narrow_markers = [
        OPS_PATH,
        OPS_CONTENT,
        PRIVATE_PATH,
        PRIVATE_CONTENT,
        REVISION_CONTENT,
        REQUEST_NAME,
        REQUEST_DESCRIPTION,
        DISCUSSION_BODY,
        REPLY_BODY,
        ATTACHMENT_FILENAME,
        INVITE_EMAIL,
        LOG_TEXT,
    ]
    .map(str::to_string)
    .to_vec();
    let mut outsider_markers = narrow_markers.clone();
    outsider_markers.extend(
        [
            VISIBLE_PATH,
            COMMIT_MESSAGE,
            TEST_OWNER_EMAIL,
            super::super::fake_github::GITHUB_FULL_NAME,
            super::super::test_repo_incarnation().incarnation_id(),
            REPO_DESCRIPTION,
            GITHUB_WORKFLOW_NAME,
            GITHUB_JOB_NAME,
        ]
        .map(str::to_string),
    );
    let write_query = format!(
        "workflow=test&git_oid={}&request_id={}",
        params["head_oid"],
        "0".repeat(32)
    );
    let write_bodies = write_bodies(&params);
    RepoA {
        app: router(state),
        params,
        absent,
        children_in_b,
        absent_children_in_b,
        queries,
        write_query,
        write_bodies,
        outsider_markers,
        narrow_markers,
    }
}
