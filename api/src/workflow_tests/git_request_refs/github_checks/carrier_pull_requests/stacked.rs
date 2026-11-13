use super::*;
use crate::workflow_tests::{
    git_request_refs::github_main_publication::set_publication, requests::start_private_request,
};
use scope_domain::requests::{RequestBase, RequestStackStatus};

struct Stack {
    parent: OwnerRequest,
    parent_head: String,
    child_id: String,
    child_head: String,
    child_branch: String,
}

impl Stack {
    async fn create(label: &str) -> Self {
        let parent = owner_request(label, &[]).await;
        let state = &parent.state;
        state
            .metadata
            .repositories()
            .set_github_review_bot_pull_requests(TEST_REPO_ID, &test_owner_id(), true, unix_now())
            .await
            .unwrap();
        let parent_head = parent.head();
        let parent_revision = state
            .metadata
            .requests()
            .latest_request_revision(&parent.request_id)
            .await
            .unwrap()
            .unwrap();
        let base = serde_json::json!({
            "kind": "request", "request_id": parent.request_id,
            "revision_id": parent_revision.id, "head_oid": parent_head,
        });
        let child_id = start_private_request(state, "stacked-child", Some(base)).await;
        push_change(
            &parent.source,
            &parent.remote,
            "refs/heads/stacked-child",
            "child.txt",
            "child work\n",
            "child change",
        )
        .unwrap();
        submit_request(router(state.clone()), &child_id, &bearer_header()).await;
        Self {
            child_head: git_head_oid(&parent.source),
            parent,
            parent_head,
            child_branch: format!("scope/requests/{child_id}"),
            child_id,
        }
    }

    fn pull(&self, branch: &str) -> serde_json::Value {
        let pulls = self.parent.fake.pulls.pulls.lock().unwrap();
        let mut from_branch = pulls.iter().filter(|pull| pull["head"]["ref"] == branch);
        let pull = from_branch.next().unwrap().clone();
        assert_eq!(from_branch.next(), None, "one pull request per branch");
        pull
    }

    fn parent_pull_request_number(&self) -> u64 {
        self.pull(&self.parent.branch())["number"].as_u64().unwrap()
    }

    async fn github_ends_parent_pull_request(&self) {
        self.parent
            .fake
            .pulls
            .close_as(self.parent_pull_request_number(), "octocat");
        self.deliver_change_of(&self.parent.branch()).await;
    }

    fn github_squash_merges_parent(&self) -> String {
        let source = &self.parent.source;
        run_git(
            Some(source),
            &["checkout", "--detach", &self.parent_head],
            "check out the parent",
        )
        .unwrap();
        let squash = squash_onto_main(&self.parent);
        self.parent.fake.pulls.merge_as(
            self.parent_pull_request_number(),
            "octocat",
            &self.parent_head,
            &squash,
        );
        squash
    }

    async fn child(&self) -> Request {
        stored_request(&self.parent.state, &self.child_id).await
    }

    fn push_child(&self, headers: &[String]) -> Result<(), String> {
        let remote = &self.parent.remote;
        let mut args = Vec::new();
        for header in headers {
            args.extend([
                "-c".to_string(),
                format!("http.{remote}.extraHeader={header}"),
            ]);
        }
        args.extend([
            "push".to_string(),
            format!(
                "--force-with-lease=refs/heads/stacked-child:{}",
                self.child_head
            ),
            remote.clone(),
            "HEAD:refs/heads/stacked-child".to_string(),
        ]);
        run_git(
            Some(&self.parent.source),
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            "push the child",
        )
        .map_err(|error| error.operator_diagnostic().to_string())
    }

    async fn deliver_change_of(&self, branch: &str) {
        let pull = self.pull(branch);
        let delivered = webhook(
            &self.parent.state,
            "pull_request",
            serde_json::json!({
                "action": "edited",
                "repository": { "id": GITHUB_REPOSITORY_ID },
                "sender": { "login": "octocat" },
                "pull_request": {
                    "number": pull["number"],
                    "html_url": pull["html_url"],
                    "state": pull["state"],
                    "head": { "ref": branch, "repo": { "id": GITHUB_REPOSITORY_ID } },
                    "base": { "ref": "main", "repo": { "id": GITHUB_REPOSITORY_ID } },
                },
            }),
            WEBHOOK_SECRET,
        )
        .await;
        assert_eq!(delivered.status(), StatusCode::NO_CONTENT);
    }

    async fn merge_parent(&self) {
        let merged = api_request(
            router(self.parent.state.clone()),
            "POST",
            &repo_request_merge(TEST_REPO_OWNER, TEST_REPO_NAME, &self.parent.request_id),
            Some(&bearer_header()),
            None,
        )
        .await;
        expect_json(merged, StatusCode::OK).await;
    }

    fn parent_branch_on_github(&self) -> Option<String> {
        self.parent.fake.branch_head(&self.parent.branch())
    }

    async fn settle_pull_requests(&self) {
        while pull_request_pass(&self.parent.state).await > 0 {}
    }

    async fn settle_pushes(&self) {
        while push_pass(&self.parent.state, unix_now()).await > 0 {}
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_merged_parents_branch_stays_until_the_childs_pull_request_has_moved_to_main() {
    let stack = Stack::create("carrier-stacked-merge").await;
    let (parent, state, fake) = (&stack.parent, &stack.parent.state, &stack.parent.fake);
    expect_json(set_publication(state, true).await, StatusCode::OK).await;
    stack.settle_pushes().await;
    stack.settle_pull_requests().await;
    let opened = stack.pull(&stack.child_branch);
    assert_eq!(opened["base"]["ref"], parent.branch());

    stack.merge_parent().await;
    stack.settle_pushes().await;
    assert!(fake.branch_head("main").is_some());
    assert_eq!(
        stack.parent_branch_on_github(),
        Some(stack.parent_head.clone())
    );

    stack.settle_pull_requests().await;
    let moved = stack.pull(&stack.child_branch);
    assert_eq!(moved["base"]["ref"], "main");
    assert_eq!(moved["state"], "open");
    assert_eq!(moved["number"], opened["number"]);
    assert_eq!(push_pass(state, FAR_FUTURE_UNIX).await, 1);
    assert_eq!(stack.parent_branch_on_github(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pull_request_scope_has_not_recorded_keeps_its_base_until_scope_moves_it() {
    let stack = Stack::create("carrier-stacked-unrecorded").await;
    let (parent, state, fake) = (&stack.parent, &stack.parent.state, &stack.parent.fake);
    expect_json(set_publication(state, true).await, StatusCode::OK).await;
    stack.settle_pushes().await;
    for _ in 0..100 {
        fake.pulls.open_from_fork("someones-fork", &parent.branch());
    }
    let landed = fake.pulls.open(&stack.child_branch, &parent.branch());

    stack.merge_parent().await;
    fake.pulls.forbidden.store(true, Ordering::SeqCst);
    stack.settle_pull_requests().await;
    fake.pulls.forbidden.store(false, Ordering::SeqCst);
    stack.settle_pushes().await;
    assert_eq!(
        stack.parent_branch_on_github(),
        Some(stack.parent_head.clone())
    );

    stack.settle_pull_requests().await;
    let moved = stack.pull(&stack.child_branch);
    assert_eq!(moved["number"], landed);
    assert_eq!(moved["base"]["ref"], "main");
    assert_eq!(moved["state"], "open");
    assert_eq!(push_pass(state, FAR_FUTURE_UNIX).await, 1);
    assert_eq!(stack.parent_branch_on_github(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_branch_under_more_pull_requests_than_scope_lists_goes_once_they_close() {
    let stack = Stack::create("carrier-stacked-unlisted").await;
    let (parent, state, fake) = (&stack.parent, &stack.parent.state, &stack.parent.fake);
    stack.settle_pushes().await;
    stack.settle_pull_requests().await;
    let from_forks = (0..1_000)
        .map(|_| fake.pulls.open_from_fork("someones-fork", &parent.branch()))
        .collect::<Vec<_>>();

    close_request(parent).await;
    stack.settle_pull_requests().await;
    assert_eq!(stack.pull(&stack.child_branch)["base"]["ref"], "main");
    stack.settle_pushes().await;
    for hour in 1..=2 {
        let rechecked = push_pass(state, FAR_FUTURE_UNIX + hour * 3_600).await;
        assert_eq!(rechecked, 1, "{hour}");
    }
    assert_eq!(
        stack.parent_branch_on_github(),
        Some(stack.parent_head.clone())
    );

    for number in from_forks {
        fake.pulls.close_as(number, "octocat");
    }
    assert_eq!(push_pass(state, FAR_FUTURE_UNIX + 3 * 3_600).await, 1);
    assert_eq!(stack.parent_branch_on_github(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disconnected_repository_keeps_a_closed_parents_branch_under_an_open_pull_request() {
    let stack = Stack::create("carrier-stacked-disconnected").await;
    let (parent, state) = (&stack.parent, &stack.parent.state);
    stack.settle_pushes().await;
    stack.settle_pull_requests().await;
    state
        .metadata
        .repositories()
        .disconnect_github_repository(TEST_REPO_ID, &test_owner_id())
        .await
        .unwrap();

    close_request(parent).await;
    stack.settle_pushes().await;
    stack.settle_pull_requests().await;
    for hour in 1..=5 {
        let rechecked = push_pass(state, FAR_FUTURE_UNIX + hour * 3_600).await;
        assert_eq!(rechecked, 1, "{hour}");
    }
    assert_eq!(
        stack.parent_branch_on_github(),
        Some(stack.parent_head.clone())
    );
    let child = stack.pull(&stack.child_branch);
    assert_eq!(child["state"], "open");
    assert_eq!(child["base"]["ref"], parent.branch());
    assert_eq!(stack.pull(&parent.branch())["state"], "closed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_child_moves_to_main_when_the_parents_pull_request_ends_and_is_not_moved_back() {
    let stack = Stack::create("carrier-stacked-parent-ended").await;
    let parent = &stack.parent;
    stack.settle_pushes().await;
    stack.settle_pull_requests().await;
    let opened = stack.pull(&stack.child_branch);
    assert_eq!(opened["base"]["ref"], parent.branch());

    stack.github_ends_parent_pull_request().await;
    stack.settle_pull_requests().await;
    assert_eq!(stack.pull(&stack.child_branch)["base"]["ref"], "main");

    stack.deliver_change_of(&stack.child_branch).await;
    stack.settle_pull_requests().await;
    let child = stack.pull(&stack.child_branch);
    assert_eq!(child["base"]["ref"], "main");
    assert_eq!(child["state"], "open");
    assert_eq!(child["number"], opened["number"]);
    stack.settle_pushes().await;
    assert_eq!(
        stack.parent_branch_on_github(),
        Some(stack.parent_head.clone())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_child_stays_on_a_parent_whose_pull_request_reopened_before_the_child_moved() {
    let stack = Stack::create("carrier-stacked-parent-reopened").await;
    let (parent, state, fake) = (&stack.parent, &stack.parent.state, &stack.parent.fake);
    stack.settle_pushes().await;
    stack.settle_pull_requests().await;

    fake.pulls.writes_unreachable.store(true, Ordering::SeqCst);
    stack.github_ends_parent_pull_request().await;
    stack.settle_pull_requests().await;
    fake.pulls.writes_unreachable.store(false, Ordering::SeqCst);
    assert_eq!(
        stack.pull(&stack.child_branch)["base"]["ref"],
        parent.branch()
    );

    fake.pulls.reopen(stack.parent_pull_request_number());
    stack.deliver_change_of(&parent.branch()).await;
    stack.settle_pull_requests().await;
    run_due_github_pull_requests(state, FAR_FUTURE_UNIX)
        .await
        .unwrap();
    assert_eq!(
        stack.pull(&stack.child_branch)["base"]["ref"],
        parent.branch()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_child_a_maintainer_moved_to_main_stays_there_while_its_parents_pull_request_is_open() {
    let stack = Stack::create("carrier-stacked-moved-to-main").await;
    let (parent, fake) = (&stack.parent, &stack.parent.fake);
    stack.settle_pushes().await;
    stack.settle_pull_requests().await;
    let opened = stack.pull(&stack.child_branch);
    assert_eq!(opened["base"]["ref"], parent.branch());

    fake.pulls
        .rebase_onto(opened["number"].as_u64().unwrap(), "main");
    stack.deliver_change_of(&stack.child_branch).await;
    stack.settle_pull_requests().await;
    let child = stack.pull(&stack.child_branch);
    assert_eq!(child["base"]["ref"], "main");
    assert_eq!(child["state"], "open");
    assert_eq!(child["number"], opened["number"]);
    assert_eq!(stack.pull(&parent.branch())["state"], "open");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_branch_stays_while_github_refuses_the_listing_for_anything_but_a_missing_permission() {
    let request = owner_request("carrier-listing-refused", &[]).await;
    let (state, fake) = (&request.state, &request.fake);
    push_pass(state, unix_now()).await;
    let head = request.head();
    assert_eq!(fake.branch_head(&request.branch()), Some(head.clone()));

    close_request(&request).await;
    let refused_by_an_ip_allow_list = &fake.pulls.forbidden_by_an_ip_allow_list;
    refused_by_an_ip_allow_list.store(true, Ordering::SeqCst);
    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert_eq!(push_pass(state, unix_now()).await, 0);
    for hour in 1..=5 {
        let rechecked = push_pass(state, FAR_FUTURE_UNIX + hour * 3_600).await;
        assert_eq!(rechecked, 1, "{hour}");
    }
    assert_eq!(fake.branch_head(&request.branch()), Some(head));

    refused_by_an_ip_allow_list.store(false, Ordering::SeqCst);
    fake.pulls.forbidden.store(true, Ordering::SeqCst);
    assert_eq!(push_pass(state, FAR_FUTURE_UNIX + 6 * 3_600).await, 1);
    assert_eq!(fake.branch_head(&request.branch()), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_squash_merged_parent_lands_and_its_child_waits_on_main_for_a_restack() {
    for main_arrives_before_the_merge_is_seen in [false, true] {
        let stack = Stack::create("carrier-stacked-squash").await;
        let (parent, state) = (&stack.parent, &stack.parent.state);
        stack.settle_pushes().await;
        stack.settle_pull_requests().await;
        let squash = stack.github_squash_merges_parent();
        if main_arrives_before_the_merge_is_seen {
            push_main(parent).await;
        }
        stack.deliver_change_of(&parent.branch()).await;
        stack.settle_pull_requests().await;
        if !main_arrives_before_the_merge_is_seen {
            assert_eq!(
                stored_request(state, &parent.request_id).await.state(),
                RequestState::Open
            );
            push_main(parent).await;
            stack.settle_pull_requests().await;
        }

        let landed = stored_request(state, &parent.request_id).await;
        assert_eq!(landed.state(), RequestState::Merged);
        assert_eq!(landed.merged_main_oid.as_deref(), Some(squash.as_str()));
        let waiting = stack.child().await;
        assert_eq!(waiting.state(), RequestState::Open);
        assert_eq!(waiting.head_oid, stack.child_head);
        assert_eq!(
            waiting.base,
            RequestBase::Main {
                review_base_oid: stack.parent_head.clone()
            }
        );
        assert_eq!(waiting.stack_status, RequestStackStatus::NeedsMainUpdate);
        assert_eq!(stack.pull(&stack.child_branch)["base"]["ref"], "main");

        let source = &parent.source;
        let child_onto = |base: &str| {
            run_git(
                Some(source),
                &[
                    "-c",
                    "user.name=Scope Test",
                    "-c",
                    "user.email=scope-test@example.test",
                    "rebase",
                    "--onto",
                    base,
                    &stack.parent_head,
                    &stack.child_head,
                ],
                "replay the child",
            )
            .unwrap();
        };
        child_onto(&stack.parent_head);
        commit_file(source, "child.txt", "more child work\n");
        let still_on_the_parents_commits = stack.push_child(&[]).unwrap_err();
        assert!(
            still_on_the_parents_commits
                .contains("request must contain current view main before recovering"),
            "{still_on_the_parents_commits}"
        );
        assert_eq!(stack.child().await, waiting);

        child_onto(&squash);
        stack
            .push_child(&[
                format!("Scope-Request-Base: main:{squash}"),
                format!("Scope-Request-Base-Version: {}", waiting.base_version),
            ])
            .unwrap();
        let restacked = stack.child().await;
        assert_eq!(restacked.stack_status, RequestStackStatus::Main);
        assert_eq!(
            restacked.base,
            RequestBase::Main {
                review_base_oid: squash.clone()
            }
        );
        assert_eq!(
            git_stdout_text(
                source,
                &["diff", "--name-only", &squash, &restacked.head_oid],
                "read the child's change"
            )
            .unwrap(),
            "child.txt\n"
        );
    }
}
