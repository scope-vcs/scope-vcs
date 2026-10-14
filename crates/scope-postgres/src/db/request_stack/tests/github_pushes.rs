use super::*;

#[tokio::test]
async fn github_push_claim_obeys_stack_gate_without_delaying_recovery() {
    for &status in RequestStackStatus::ALL {
        let store = crate::db::requests::tests::postgres_store();
        let mut request = start(
            &store,
            "contribution",
            RequestBase::Main {
                review_base_oid: "base".into(),
            },
            2,
        )
        .await;
        request.view = ViewId::public();
        let blocked = match status {
            RequestStackStatus::Main
            | RequestStackStatus::WaitingForParent
            | RequestStackStatus::ParentExtended => false,
            RequestStackStatus::ParentRewritten
            | RequestStackStatus::ParentClosed
            | RequestStackStatus::AncestorBlocked
            | RequestStackStatus::IntegrationConflict
            | RequestStackStatus::NeedsMainUpdate => true,
        };
        if !matches!(
            status,
            RequestStackStatus::Main
                | RequestStackStatus::IntegrationConflict
                | RequestStackStatus::NeedsMainUpdate
        ) {
            let parent = start(&store, "parent", request.base.clone(), 8).await;
            request.base = pin(&store, &parent).await;
            request.stack_depth = 2;
        }
        request.stack_status = status;
        super::super::super::request_rows::save_request_row(store.db.as_ref(), &request)
            .await
            .unwrap();
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let destination = scope_domain::requests::GitHubPushDestination {
            installation_id: 7,
            github_repository_id: 42,
            github_full_name: "octo/checks".into(),
        };
        let branch = scope_domain::requests::GitHubBranch::Request(request.id.clone());
        crate::db::github_pushes::queue_github_push(
            store.db.as_ref(),
            &request.repo_id,
            &branch,
            Some(&request.head_oid),
            &destination,
            t,
        )
        .await
        .unwrap();
        let claimed = store
            .requests()
            .claim_due_github_pushes("initial", t, t + 2700, 10)
            .await
            .unwrap();
        assert_eq!(
            claimed.is_empty(),
            blocked,
            "push claim gate for {status:?}"
        );
        if !blocked {
            assert_eq!(claimed.len(), 1);
            continue;
        }
        request.stack_status = RequestStackStatus::Main;
        request.base = RequestBase::Main {
            review_base_oid: "base".into(),
        };
        request.stack_depth = 1;
        request.head_oid = "a".repeat(40);
        request.git_snapshot.as_mut().unwrap().git_oid = request.head_oid.clone();
        super::super::super::request_rows::save_request_row(store.db.as_ref(), &request)
            .await
            .unwrap();
        crate::db::github_pushes::queue_github_push(
            store.db.as_ref(),
            &request.repo_id,
            &branch,
            Some(&request.head_oid),
            &destination,
            t + 1,
        )
        .await
        .unwrap();
        let claimed = store
            .requests()
            .claim_due_github_pushes("recovered", t + 1, t + 2701, 10)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(
            claimed[0].target_oid.as_deref(),
            Some(request.head_oid.as_str())
        );
    }
}

#[tokio::test]
async fn github_push_claim_and_completion_wait_for_rewritten_dependencies_then_recover() {
    let store = crate::db::requests::tests::postgres_store();
    let parent = start(
        &store,
        "parent",
        RequestBase::Main {
            review_base_oid: "base".into(),
        },
        2,
    )
    .await;
    let child = start(&store, "child", pin(&store, &parent).await, 6).await;
    let grandchild = start(&store, "grandchild", pin(&store, &child).await, 10).await;
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let destination = scope_domain::requests::GitHubPushDestination {
        installation_id: 7,
        github_repository_id: 42,
        github_full_name: "octo/checks".into(),
    };
    for request in [&child, &grandchild] {
        crate::db::github_pushes::queue_github_push(
            store.db.as_ref(),
            &request.repo_id,
            &scope_domain::requests::GitHubBranch::Request(request.id.clone()),
            Some(&request.head_oid),
            &destination,
            t,
        )
        .await
        .unwrap();
    }
    let claimed = store
        .requests()
        .claim_due_github_pushes("claim", t, t + 100, 10)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 2);
    let child_push = claimed
        .iter()
        .find(|push| push.branch == scope_domain::requests::GitHubBranch::Request(child.id.clone()))
        .unwrap();
    let publication = input(&parent, "parent_rewritten", "rewrite_parent", t + 1);
    let child_facts = vec![fact(&publication.new_head_oid, &child, false)];
    let parent = store
        .requests()
        .record_request_revision_with_stack(publication, parent.base_version, None, child_facts)
        .await
        .unwrap()
        .request;
    assert_eq!(
        store
            .requests()
            .github_push_standing(&child_push.id, "claim")
            .await
            .unwrap(),
        crate::db::GitHubPushStanding::Lost
    );
    assert!(
        store
            .requests()
            .finish_github_push(
                &child_push.id,
                "claim",
                crate::db::GitHubJobOutcome::Succeeded(()),
                t + 2
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .requests()
            .claim_due_github_pushes("blocked", t + 101, t + 200, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let mut publication = input(&child, "restacked_child", "restack_child", t + 102);
    publication.new_base = Some(pin(&store, &parent).await);
    let child_facts = vec![fact(&publication.new_head_oid, &grandchild, false)];
    let recovered = store
        .requests()
        .record_request_revision_with_stack(
            publication,
            child.base_version,
            parent_facts(&store, &child).await,
            child_facts,
        )
        .await
        .unwrap()
        .request;
    crate::db::github_pushes::queue_github_push(
        store.db.as_ref(),
        &recovered.repo_id,
        &scope_domain::requests::GitHubBranch::Request(recovered.id.clone()),
        Some(&recovered.head_oid),
        &destination,
        t + 102,
    )
    .await
    .unwrap();
    let resumed = store
        .requests()
        .claim_due_github_pushes("resumed", t + 102, t + 200, 10)
        .await
        .unwrap();
    assert_eq!(resumed.len(), 1);
    assert_eq!(
        resumed[0].target_oid.as_deref(),
        Some(recovered.head_oid.as_str())
    );
    assert_eq!(
        load(&store, &grandchild.id).await.stack_status,
        RequestStackStatus::ParentRewritten
    );
}
