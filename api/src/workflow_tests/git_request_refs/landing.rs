use super::super::main_pool::main_pool_packs;
use super::*;

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
        let (state, source, base_head) =
            super::super::push_intent_completion::published_git_fixture(label).await;
        let app = router(state.clone());
        let bearer = bearer_header();
        let (origin, server) = spawn_test_server(&state).await;
        let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
        configure_bearer_header(&source, &remote, &bearer);

        let mut request_ids = Vec::new();
        for name in names {
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
                &format!("{name}.txt"),
                "landed work\n",
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
                Some("landed work\n")
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_merge_waits_for_the_earlier_merge_to_commit_and_then_lands() {
    let names = ["left", "right"];
    let landing = DisjointRequests::open("same-main-landing", &names).await;
    let admin = landing.state.metadata.admin();
    let held = admin.lock_aggregates_for_tests().await.unwrap();

    let merges = landing
        .request_ids
        .iter()
        .map(|id| tokio::spawn(landing.merge(id)))
        .collect::<Vec<_>>();
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        while admin.lock_waiters_for_tests().await.unwrap() < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("one merge must wait to commit while the other waits for its turn");
    held.rollback().await.unwrap();

    let mut merged = Vec::new();
    for merge in merges {
        merged.push(merge.await.unwrap());
    }
    landing.assert_all_landed(&names, merged).await;
}
