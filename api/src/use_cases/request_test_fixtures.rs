use crate::state::AppState;
use scope_domain::{
    account::UserAccount,
    repository::{
        RepoLifecycleState, Repository,
        collaboration::{RepositoryMember, RepositoryMemberPermissions},
    },
    requests::{RecordWorkingRequestUploadInput, RequestActorRole, StartRequestInput},
    views::ViewId,
};
use scope_postgres::db::{CatalogFixture, MetadataStore};

pub(crate) fn user(id: &str, handle: &str) -> UserAccount {
    UserAccount {
        id: id.to_string(),
        handle: handle.to_string(),
        email: format!("{handle}@example.com"),
        email_verified: true,
    }
}

pub(crate) fn seeded(public_user_is_member: bool) -> AppState {
    let state = AppState::test_state();
    let owner = user("user_owner", "owner");
    let mut repo = Repository::new(&owner, "repo", ViewId::public(), "repoi_test").unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    if public_user_is_member {
        repo.collaboration.members.push(RepositoryMember {
            repo_id: "owner/repo".to_string(),
            user_id: "user_public".to_string(),
            permissions: RepositoryMemberPermissions {
                can_push: true,
                can_change_file_visibility: true,
                view: ViewId::private(),
            },
            created_at_unix: 1,
            updated_at_unix: 1,
        });
    }
    let mut catalog = CatalogFixture::default();
    for account in [
        owner,
        user("user_public", "public"),
        user("user_guest", "guest"),
    ] {
        catalog.users.insert(account.id.clone(), account);
    }
    catalog.repositories.insert(repo.record.id.clone(), repo);
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(catalog)
        .unwrap();
    state
}

pub(crate) fn public_start_input() -> StartRequestInput {
    StartRequestInput {
        id: "req_1".to_string(),
        repo_id: "owner/repo".to_string(),
        name: "fix-parser".to_string(),
        author_user_id: "user_public".to_string(),
        title: Some("Fix parser crash".to_string()),
        author_role: RequestActorRole::Public,
        author_view: ViewId::public(),
        view: ViewId::public(),
        base: scope_domain::requests::RequestBase::Main {
            review_base_oid: "base".to_string(),
        },
        main_ancestor_oid: "base".to_string(),
        event_id: "event_started".to_string(),
        now_unix: 2,
    }
}

pub(crate) fn public_upload_input() -> RecordWorkingRequestUploadInput {
    RecordWorkingRequestUploadInput {
        request_id: "req_1".to_string(),
        actor_user_id: "user_public".to_string(),
        expected_old_head_oid: None,
        new_head_oid: "head".to_string(),
        now_unix: 3,
    }
}

pub(crate) async fn start_public_request(store: &MetadataStore) {
    store
        .requests()
        .start_request(public_start_input())
        .await
        .unwrap();
    store
        .requests()
        .record_working_request_upload(public_upload_input())
        .await
        .unwrap();
}
