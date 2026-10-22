use super::super::main_pool::main_pool_packs;
use super::super::merge_commits::{commit_blocked_behind_held_lock, taken_while_committing};
use super::*;
use std::time::Duration;
use tokio::task::JoinHandle;

struct DisjointRequests {
    state: AppState,
    app: axum::Router,
    bearer: String,
    request_ids: Vec<String>,
    _source: TempGitRepo,
    _server: TestServer,
}

impl DisjointRequests {
    async fn open(label: &str, names: &[&str]) -> Self {
        let files = names
            .iter()
            .map(|name| (*name, format!("{name}.txt")))
            .collect::<Vec<_>>();
        let files = files
            .iter()
            .map(|(name, file)| (*name, file.as_str()))
            .collect::<Vec<_>>();
        Self::open_with_files(label, &files).await
    }

    async fn open_with_files(label: &str, requests: &[(&str, &str)]) -> Self {
        let (state, source, base_head) =
            super::super::push_intent_completion::published_git_fixture(label).await;
        let app = router(state.clone());
        let bearer = bearer_header();
        let (origin, server) = spawn_test_server(&state).await;
        let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
        configure_bearer_header(&source, &remote, &bearer);

        let mut request_ids = Vec::new();
        for (name, file) in requests {
            let started = api_request(
                app.clone(),
                "POST",
                &format!("/v1/repos/{TEST_REPO_ID}/requests"),
                Some(&bearer),
                Some(&serde_json::json!({ "name": name, "view": "private" }).to_string()),
            )
            .await;
            assert_eq!(started.status(), StatusCode::OK);
            let id = response_json(started).await["request"]["id"]
                .as_str()
                .unwrap()
                .to_string();
            run_git(
                Some(&source),
                &["checkout", "--detach", &base_head],
                "pick base",
            )
            .unwrap();
            push_change(
                &source,
                &remote,
                &format!("refs/heads/{name}"),
                file,
                &format!("{name} work\n"),
                name,
            )
            .unwrap();
            submit_request(app.clone(), &id, &bearer).await;
            request_ids.push(id);
        }
        Self {
            state,
            app,
            bearer,
            request_ids,
            _source: source,
            _server: server,
        }
    }

    fn merge(&self, request_id: &str) -> impl std::future::Future<Output = Response> + 'static {
        let app = self.app.clone();
        let bearer = self.bearer.clone();
        let route = scope_api_contract::routes::repo_request_merge(
            TEST_REPO_OWNER,
            TEST_REPO_NAME,
            request_id,
        );
        async move { api_request(app, "POST", &route, Some(&bearer), None).await }
    }

    async fn assert_all_landed(&self, names: &[&str], merges: Vec<Response>) {
        for merged in merges {
            assert_eq!(
                expect_json(merged, StatusCode::OK).await["request"]["state"],
                "Merged"
            );
        }
        for name in names {
            assert_eq!(
                live_file_content(&self.state, &format!("/{name}.txt"))
                    .await
                    .as_deref(),
                Some(format!("{name} work\n").as_str())
            );
        }
        let sequences = main_pool_packs(&self.state)
            .await
            .iter()
            .map(|pack| (pack.first_sequence, pack.last_sequence))
            .collect::<Vec<_>>();
        let landed = u64::try_from(names.len()).unwrap();
        assert_eq!(
            sequences,
            (1..=landed + 1)
                .map(|sequence| (sequence, sequence))
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_merges_of_disjoint_requests_all_land() {
    let names = ["left", "middle", "right"];
    let landing = DisjointRequests::open("concurrent-landing", &names).await;

    let merges = landing.request_ids.iter().map(|id| landing.merge(id));
    let merged = futures_util::future::join_all(merges).await;

    landing.assert_all_landed(&names, merged).await;
}

impl DisjointRequests {
    async fn queue_behind_held_turn(&self, request_ids: &[String]) -> Vec<JoinHandle<Response>> {
        let committer = self.state.merge_committer.clone();
        let mut merges = Vec::new();
        for (queued, id) in request_ids.iter().enumerate() {
            merges.push(tokio::spawn(self.merge(id)));
            tokio::time::timeout(Duration::from_secs(60), async {
                while committer.queued(TEST_REPO_ID) <= queued {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("every merge must queue behind the held merge turn");
        }
        merges
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn merges_queued_on_one_replica_commit_in_one_transaction() {
    let names = ["left", "middle", "right"];
    let landing = DisjointRequests::open("batched-landing", &names).await;
    let admin = landing.state.metadata.admin();
    let repositories = landing.state.metadata.repositories();
    let turn = repositories.acquire_merge_turn(TEST_REPO_ID).await.unwrap();
    let held = admin.lock_aggregates_for_tests().await.unwrap();

    let merges = landing.queue_behind_held_turn(&landing.request_ids).await;
    turn.release().await;
    tokio::time::timeout(Duration::from_secs(60), async {
        while landing.state.merge_committer.queued(TEST_REPO_ID) > 0
            || admin.lock_waiters_for_tests().await.unwrap() < 1
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the committer must take every queued merge into one waiting transaction");
    assert_eq!(admin.lock_waiters_for_tests().await.unwrap(), 1);
    held.rollback().await.unwrap();

    let mut merged = Vec::new();
    for merge in merges {
        merged.push(merge.await.unwrap());
    }
    landing.assert_all_landed(&names, merged).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_conflicting_merge_fails_alone_and_the_rest_of_its_batch_lands() {
    let landing = DisjointRequests::open_with_files(
        "batched-conflict",
        &[
            ("left", "left.txt"),
            ("first-claim", "claimed.txt"),
            ("second-claim", "claimed.txt"),
            ("right", "right.txt"),
        ],
    )
    .await;
    let turn = landing
        .state
        .metadata
        .repositories()
        .acquire_merge_turn(TEST_REPO_ID)
        .await
        .unwrap();
    let merges = landing.queue_behind_held_turn(&landing.request_ids).await;
    turn.release().await;

    let mut merged = Vec::new();
    for merge in merges {
        merged.push(merge.await.unwrap());
    }
    let second_claim = merged.remove(2);
    assert_eq!(second_claim.status(), StatusCode::CONFLICT);
    for merged in merged {
        assert_eq!(
            expect_json(merged, StatusCode::OK).await["request"]["state"],
            "Merged"
        );
    }
    assert_eq!(
        live_file_content(&landing.state, "/claimed.txt")
            .await
            .as_deref(),
        Some("first-claim work\n")
    );
    for name in ["left", "right"] {
        assert_eq!(
            live_file_content(&landing.state, &format!("/{name}.txt"))
                .await
                .as_deref(),
            Some(format!("{name} work\n").as_str())
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_merge_whose_caller_disconnected_still_announces_its_change() {
    let landing = DisjointRequests::open("disconnected-merge", &["left"]).await;
    let mut events = landing.state.repo_events.subscribe(TEST_REPO_ID);
    let turn = landing
        .state
        .metadata
        .repositories()
        .acquire_merge_turn(TEST_REPO_ID)
        .await
        .unwrap();
    let caller = landing
        .queue_behind_held_turn(&landing.request_ids)
        .await
        .pop()
        .unwrap();
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    turn.release().await;

    tokio::time::timeout(Duration::from_secs(30), events.recv())
        .await
        .expect("the landed merge must announce its repository change")
        .unwrap();
    assert_eq!(
        landing
            .state
            .metadata
            .requests()
            .request_by_id(&landing.request_ids[0])
            .await
            .unwrap()
            .unwrap()
            .state(),
        scope_domain::requests::RequestState::Merged
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_merge_arriving_while_a_batch_commits_is_prepared_before_it_commits() {
    let names = ["first", "second"];
    let landing = DisjointRequests::open("pipelined-landing", &names).await;
    let admin = landing.state.metadata.admin();
    let held = admin.lock_aggregates_for_tests().await.unwrap();

    let mut merges = vec![
        commit_blocked_behind_held_lock(&landing.state, landing.merge(&landing.request_ids[0]))
            .await,
    ];
    merges.extend(
        taken_while_committing(&landing.state, vec![landing.merge(&landing.request_ids[1])]).await,
    );
    assert_eq!(admin.lock_waiters_for_tests().await.unwrap(), 1);
    held.rollback().await.unwrap();

    let mut merged = Vec::new();
    for merge in merges {
        merged.push(merge.await.unwrap());
    }
    landing.assert_all_landed(&names, merged).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_merge_prepared_on_a_batch_that_fails_to_land_lands_on_the_committed_main() {
    let landing = DisjointRequests::open("pipelined-rejection", &["first", "second"]).await;
    let admin = landing.state.metadata.admin();
    let held = admin.lock_aggregates_for_tests().await.unwrap();

    let first =
        commit_blocked_behind_held_lock(&landing.state, landing.merge(&landing.request_ids[0]))
            .await;
    let second =
        taken_while_committing(&landing.state, vec![landing.merge(&landing.request_ids[1])])
            .await
            .pop()
            .unwrap();
    admin
        .execute_for_tests(&format!(
            "UPDATE scope_requests SET head_oid = review_base_oid WHERE id = '{}'",
            landing.request_ids[0]
        ))
        .await
        .unwrap();
    held.rollback().await.unwrap();

    assert_eq!(first.await.unwrap().status(), StatusCode::CONFLICT);
    assert_eq!(
        expect_json(second.await.unwrap(), StatusCode::OK).await["request"]["state"],
        "Merged"
    );
    assert_eq!(live_file_content(&landing.state, "/first.txt").await, None);
    assert_eq!(
        live_file_content(&landing.state, "/second.txt")
            .await
            .as_deref(),
        Some("second work\n")
    );
}
