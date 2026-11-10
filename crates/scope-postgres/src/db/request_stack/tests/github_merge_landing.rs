use super::{
    carrier_pull_requests::{carried_stack, intent, open_onto},
    *,
};
use crate::db::{
    GitHubJobOutcome, GitHubPullRequestFinish, MainContains,
    github_pull_requests::tests::{
        merge_landing::{MAIN, SQUASH, squash_merged_on_github, squash_seen_in},
        oid,
    },
    request_merge::{LandedRequestCandidate, LandedRequestCompletion, complete_landed_requests},
    request_rows::request_by_id,
};
use scope_domain::requests::{GitHubPullRequest, RequestState};
use sea_orm::ConnectionTrait;

fn landing_fact(
    parent: &Request,
    child: &Request,
    main_contains_pin: bool,
) -> RequestStackChildFacts {
    RequestStackChildFacts {
        main_contains_pin,
        ..fact(&parent.head_oid, child, true)
    }
}

async fn claim_carrier_of(
    store: &MetadataStore,
    parent: &Request,
    token: &str,
    at: u64,
) -> GitHubPullRequest {
    store
        .requests()
        .claim_due_github_pull_requests(token, at, at + 100, 10)
        .await
        .unwrap()
        .into_iter()
        .find(|carrier| carrier.request_id == parent.id)
        .unwrap()
}

#[tokio::test]
async fn a_parent_merged_on_github_lands_with_every_child_moved_to_main_or_not_at_all() {
    let store = &crate::db::requests::tests::postgres_store();
    let (parent, [first, second]) = carried_stack(store, ["child-a", "child-b"]).await;
    let grandchild = start(store, "grandchild", pin(store, &first).await, 20).await;
    store
        .db
        .execute_unprepared(&format!(
            "INSERT INTO scope_git_heads (repo_id, head_oid, push_sequence, change_version,
                frontier_digest, main_advanced_by_user_id)
             VALUES ('owner/repo', '{}', 1, 1, 'frontier', 'user_owner')",
            oid(MAIN)
        ))
        .await
        .unwrap();
    let merged = squash_merged_on_github(
        claim_carrier_of(store, &parent, "claim_1", 30).await,
        &parent.head_oid,
    );
    let finish = |carrier: GitHubPullRequest, token: &'static str, main, children| async move {
        store
            .requests()
            .finish_github_pull_request(
                &carrier,
                token,
                GitHubJobOutcome::Succeeded(()),
                Some(squash_seen_in(main, children)),
                31,
            )
            .await
    };
    let squashed = |children: &[&Request]| {
        children
            .iter()
            .map(|child| landing_fact(&parent, child, false))
            .collect::<Vec<_>>()
    };

    let prepared_before_the_second_child = squashed(&[&first]);
    let error = finish(
        merged.clone(),
        "claim_1",
        MAIN,
        prepared_before_the_second_child,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
    let unrecorded =
        crate::db::github_pull_requests::github_pull_request(store.db.as_ref(), &parent.id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(unrecorded.pull_request, None);

    let main_moved = finish(merged, "claim_1", 'b', squashed(&[&first, &second])).await;
    assert_eq!(main_moved.unwrap(), GitHubPullRequestFinish::Recorded);
    assert_eq!(load(store, &parent.id).await.state(), RequestState::Open);
    for untouched in [&first, &second, &grandchild] {
        assert_eq!(&load(store, &untouched.id).await, untouched);
    }
    let asking_again = claim_carrier_of(store, &parent, "claim_2", 32).await;

    let tx = store.db.begin().await.unwrap();
    acquire_aggregate_lock(&tx, "repository", &parent.repo_id)
        .await
        .unwrap();
    let with_a_merge_commit = complete_landed_requests(
        &tx,
        &parent.repo_id,
        &oid(MAIN),
        scope_domain::requests::MainAdvanceKind::Push,
        LandedRequestCompletion {
            actor_user_id: "user_owner".into(),
            candidates: vec![LandedRequestCandidate {
                request_id: parent.id.clone(),
                head_oid: parent.head_oid.clone(),
                stack_children: [&first, &second]
                    .map(|child| landing_fact(&parent, child, true))
                    .into(),
                main_contains: MainContains::GitHubMergeCommit(oid(SQUASH)),
            }],
        },
        33,
    )
    .await
    .unwrap();
    assert_eq!(with_a_merge_commit, 1);
    for child in [&first, &second] {
        let descending_from_main = request_by_id(&tx, &child.id).await.unwrap().unwrap();
        assert_eq!(descending_from_main.stack_status, RequestStackStatus::Main);
        assert_eq!(descending_from_main.stack_depth, 1);
    }
    tx.rollback().await.unwrap();

    let landed = finish(asking_again, "claim_2", MAIN, squashed(&[&first, &second])).await;
    assert_eq!(landed.unwrap(), GitHubPullRequestFinish::RecordedAndLanded);
    let parent = load(store, &parent.id).await;
    assert_eq!(parent.state(), RequestState::Merged);
    assert_eq!(parent.merged_main_oid, Some(oid(MAIN)));
    for child in [&first, &second] {
        let on_main = load(store, &child.id).await;
        assert_eq!(
            on_main.base,
            RequestBase::Main {
                review_base_oid: parent.head_oid.clone()
            }
        );
        assert_eq!(on_main.stack_depth, 1);
        assert_eq!(on_main.stack_status, RequestStackStatus::NeedsMainUpdate);
        assert_eq!(on_main.head_oid, child.head_oid);
        assert_eq!(
            intent(store, &child.id).await,
            open_onto("main", &child.head_oid)
        );
    }
    let below_a_blocked_child = load(store, &grandchild.id).await;
    assert_eq!(below_a_blocked_child.base, grandchild.base);
    assert_eq!(below_a_blocked_child.stack_depth, 2);
    assert_eq!(
        below_a_blocked_child.stack_status,
        RequestStackStatus::AncestorBlocked
    );

    let mut merged_without_a_restack = squash_merged_on_github(
        claim_carrier_of(store, &second, "claim_3", 200).await,
        &second.head_oid,
    );
    let scope_domain::requests::GitHubPullRequestState::Merged(merge) =
        &mut merged_without_a_restack
            .pull_request
            .as_mut()
            .unwrap()
            .state
    else {
        unreachable!()
    };
    merge.merge_commit_oid = oid('e');
    store
        .db
        .execute_unprepared(&format!(
            "UPDATE scope_git_heads SET head_oid = '{}' WHERE repo_id = 'owner/repo'",
            oid('e')
        ))
        .await
        .unwrap();
    let child_landed = store
        .requests()
        .finish_github_pull_request(
            &merged_without_a_restack,
            "claim_3",
            GitHubJobOutcome::Succeeded(()),
            Some(crate::db::GitHubMergeCheckedAgainstMain {
                main: scope_domain::requests::GitHubMergeMainCheck {
                    main_oid: oid('e'),
                    merge_commit_oid: oid('e'),
                    main_contains_merge: true,
                },
                stack_children: Vec::new(),
            }),
            201,
        )
        .await;
    assert_eq!(
        child_landed.unwrap(),
        GitHubPullRequestFinish::RecordedAndLanded
    );
    assert_eq!(load(store, &second.id).await.state(), RequestState::Merged);
}
