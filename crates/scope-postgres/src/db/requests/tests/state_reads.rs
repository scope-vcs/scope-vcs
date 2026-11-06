use super::*;
use crate::db::{
    AddRequestInviteeCommand, SubmitRequestCommand,
    test_support::{fixtures, read_budget::assert_round_trips},
};
use crate::error::PostgresErrorKind;
use scope_domain::repository::collaboration::{RepositoryMember, RepositoryMemberPermissions};

#[tokio::test]
async fn state_reads_preserve_viewer_visibility_and_bound_round_trips() {
    let store = postgres_store();
    let mut repo = store
        .repositories()
        .repository_for_tests("owner/repo")
        .await
        .unwrap()
        .unwrap();
    repo.graph.commits.push(fixtures::canonical_commit(
        "user_owner",
        0,
        "/public.txt",
        ViewId::public(),
    ));
    for (user_id, view) in [
        ("member", ViewId::private()),
        ("restricted", ViewId::public()),
    ] {
        repo.collaboration.members.push(RepositoryMember {
            repo_id: repo.record.id.clone(),
            user_id: user_id.into(),
            permissions: RepositoryMemberPermissions {
                view,
                ..Default::default()
            },
            created_at_unix: 1,
            updated_at_unix: 1,
        });
    }
    repo.bump_content_version();
    store
        .repositories()
        .replace_repository_for_tests(repo.clone())
        .await
        .unwrap();
    start_public_request(&store).await;
    let requests = store.requests();
    requests
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_public".into(),
            target_handle: "guest".into(),
            now_unix: 4,
        })
        .await
        .unwrap();
    let draft_error = requests
        .request_state_snapshot("owner", "repo", "req_1", None)
        .await
        .err()
        .unwrap();
    assert_eq!(draft_error.message, "request not found");
    requests
        .submit_request(SubmitRequestCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_public".into(),
            event_id: "submitted".into(),
            now_unix: 5,
        })
        .await
        .unwrap();
    let expected = requests.request_for_tests("req_1").await.unwrap().unwrap();
    for viewer in [
        Some("user_owner"),
        Some("member"),
        Some("restricted"),
        Some("user_public"),
        Some("user_guest"),
        None,
    ] {
        let snapshot = assert_round_trips(
            12,
            requests.request_state_snapshot("owner", "repo", "req_1", viewer),
        )
        .await
        .unwrap();
        assert_eq!(snapshot.request, expected);
        assert_eq!(snapshot.is_invitee, viewer == Some("user_guest"));
        assert_eq!(snapshot.invitees[0].user.id, "user_guest");
        assert_eq!(
            snapshot.repository.access,
            viewer
                .map(|id| repo.access_for_user_id(id))
                .unwrap_or_else(scope_domain::repository::access::RepositoryAccess::public)
        );
        assert_eq!(snapshot.viewer_user_id.as_deref(), viewer);
        assert_eq!(
            snapshot.revision,
            requests.latest_request_revision("req_1").await.unwrap()
        );
        assert!(snapshot.evaluation.is_none());
        assert!(snapshot.auto_merge.is_none());
        assert!(snapshot.github_connection.is_none());
    }
    for (owner, name, id, message) in [
        ("missing", "repo", "req_1", "repo not found"),
        ("owner", "repo", "missing", "request not found"),
    ] {
        let error = requests
            .request_state_snapshot(owner, name, id, Some("user_owner"))
            .await
            .err()
            .unwrap();
        assert_eq!(error.kind, PostgresErrorKind::NotFound);
        assert_eq!(error.message, message);
    }
    let mut private = public_start_input();
    private.id = "req_private".into();
    private.name = "private".into();
    private.author_user_id = "user_owner".into();
    private.author_role = RequestActorRole::Owner;
    private.author_view = ViewId::private();
    private.view = ViewId::private();
    private.event_id = "private_started".into();
    requests.start_request(private).await.unwrap();
    for viewer in [
        Some("user_owner"),
        Some("member"),
        Some("restricted"),
        Some("user_guest"),
        None,
    ] {
        let result = requests
            .request_state_snapshot("owner", "repo", "req_private", viewer)
            .await;
        if matches!(viewer, Some("user_owner" | "member")) {
            assert_eq!(result.unwrap().request.id, "req_private");
        } else {
            let error = result.err().unwrap();
            assert_eq!(error.kind, PostgresErrorKind::NotFound);
            assert_eq!(error.message, "request not found");
        }
    }
}
