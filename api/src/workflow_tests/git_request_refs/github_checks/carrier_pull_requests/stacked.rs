use super::*;
use crate::workflow_tests::{
    git_request_refs::github_main_publication::set_publication, requests::start_private_request,
};

struct Stack {
    parent: OwnerRequest,
    parent_head: String,
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
            parent,
            parent_head,
            child_branch: format!("scope/requests/{child_id}"),
        }
    }

    fn pull(&self, branch: &str) -> serde_json::Value {
        let pulls = self.parent.fake.pulls.pulls.lock().unwrap();
        let mut from_branch = pulls.iter().filter(|pull| pull["head"]["ref"] == branch);
        let pull = from_branch.next().unwrap().clone();
        assert_eq!(from_branch.next(), None, "one pull request per branch");
        pull
    }

    async fn github_ends_parent_pull_request(&self) {
        let number = self.pull(&self.parent.branch())["number"].as_u64().unwrap();
        self.parent.fake.pulls.close_as(number, "octocat");
        self.deliver_change_of(&self.parent.branch()).await;
    }

    async fn deliver_change_of(&self, branch: &str) {
        let delivered = webhook(
            &self.parent.state,
            "pull_request",
            serde_json::json!({
                "action": "edited",
                "repository": { "id": GITHUB_REPOSITORY_ID },
                "sender": { "login": "octocat" },
                "pull_request": {
                    "number": self.pull(branch)["number"],
                    "html_url": self.pull(branch)["html_url"],
                    "state": self.pull(branch)["state"],
                    "head": { "ref": branch, "repo": { "id": GITHUB_REPOSITORY_ID } },
                    "base": { "ref": "main", "repo": { "id": GITHUB_REPOSITORY_ID } },
                },
            }),
            WEBHOOK_SECRET,
        )
        .await;
        assert_eq!(delivered.status(), StatusCode::NO_CONTENT);
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

    let merged = api_request(
        router(state.clone()),
        "POST",
        &repo_request_merge(TEST_REPO_OWNER, TEST_REPO_NAME, &parent.request_id),
        Some(&bearer_header()),
        None,
    )
    .await;
    expect_json(merged, StatusCode::OK).await;
    stack.settle_pushes().await;
    assert!(fake.branch_head("main").is_some());
    assert_eq!(
        fake.branch_head(&parent.branch()),
        Some(stack.parent_head.clone())
    );

    stack.github_ends_parent_pull_request().await;
    stack.settle_pull_requests().await;
    let moved = stack.pull(&stack.child_branch);
    assert_eq!(moved["base"]["ref"], "main");
    assert_eq!(moved["state"], "open");
    assert_eq!(moved["number"], opened["number"]);
    stack.settle_pushes().await;
    assert_eq!(fake.branch_head(&parent.branch()), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_child_moves_to_main_when_the_parents_pull_request_ends_and_is_not_moved_back() {
    let stack = Stack::create("carrier-stacked-parent-ended").await;
    let (parent, fake) = (&stack.parent, &stack.parent.fake);
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
        fake.branch_head(&parent.branch()),
        Some(stack.parent_head.clone())
    );
}
