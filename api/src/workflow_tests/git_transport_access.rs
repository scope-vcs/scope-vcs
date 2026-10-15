use super::*;
use crate::config::AWAITING_FIRST_PUSH_GIT_ERROR;

const READY_PUSH_TOKEN: &str = "scope_git_test";
const MEMBER_SUBJECT: &str = "user_member";
const MEMBER_EMAIL: &str = "member@example.com";

#[derive(Clone, Copy)]
enum ReadRepo {
    PrivateOnly,
    AwaitingFirstPush,
    AwaitingFirstPushWithMember,
}

#[derive(Clone, Copy)]
enum GitReader {
    Anonymous,
    Owner,
    Member,
}

#[derive(Clone, Copy)]
enum PushCredential {
    Token(&'static str),
    FirstPushSecret,
    MemberSession,
}

#[derive(Clone, Copy)]
enum ReceiveOutcome {
    ReadyMember,
    FirstPush,
    Rejected(StatusCode, &'static str),
}

async fn private_only_repo(state: &AppState) {
    let mut repo = test_repo(&test_owner_id());
    repo.repo_config = repo_config(ViewId::private());
    repo.policy = Policy::new(ViewId::private());
    repo.graph.commits.push(LogicalCommit {
        occurred_at_unix: None,
        id: "rv1".to_string(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: "rv1".to_string(),
        },
        author_id: repo.record.owner_user_id.clone(),
        message: "initial".to_string(),
        changes: vec![FileChange {
            label: ViewId::private(),
            path: ScopePath::parse("/secret.txt").unwrap(),
            old_content: None,
            new_content: Some(source_blob(state, "secret")),
        }],
    });
    replace_test_repo(state, repo).await;
}

async fn insert_member(state: &AppState, permissions: RepositoryMemberPermissions) -> String {
    let user_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", MEMBER_SUBJECT);
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(user_id.clone(), "member", MEMBER_EMAIL))
        .await
        .unwrap();
    let member_id = user_id.clone();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, move |repo| {
            repo.collaboration.members.push(test_repository_member(
                TEST_REPO_ID,
                member_id,
                permissions,
            ));
        })
        .await
        .unwrap();
    user_id
}

async fn await_first_push(state: &AppState) {
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.record.lifecycle_state = RepoLifecycleState::AwaitingFirstPush;
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn git_reads_reveal_repository_state_only_to_permitted_readers() {
    let credentials_required = "Git credentials required";
    for (label, repo_setup, reader, repo_name, view, status, message) in [
        (
            "anonymous missing repo",
            ReadRepo::PrivateOnly,
            GitReader::Anonymous,
            "missing",
            ViewId::public(),
            StatusCode::UNAUTHORIZED,
            credentials_required,
        ),
        (
            "anonymous private-only repo",
            ReadRepo::PrivateOnly,
            GitReader::Anonymous,
            TEST_REPO_NAME,
            ViewId::public(),
            StatusCode::UNAUTHORIZED,
            credentials_required,
        ),
        (
            "owner missing repo",
            ReadRepo::PrivateOnly,
            GitReader::Owner,
            "missing",
            ViewId::private(),
            StatusCode::NOT_FOUND,
            "repo owner/missing not found",
        ),
        (
            "owner unpublished repo",
            ReadRepo::AwaitingFirstPush,
            GitReader::Owner,
            TEST_REPO_NAME,
            ViewId::private(),
            StatusCode::FORBIDDEN,
            AWAITING_FIRST_PUSH_GIT_ERROR,
        ),
        (
            "anonymous unpublished repo",
            ReadRepo::AwaitingFirstPush,
            GitReader::Anonymous,
            TEST_REPO_NAME,
            ViewId::public(),
            StatusCode::UNAUTHORIZED,
            credentials_required,
        ),
        (
            "member unpublished repo",
            ReadRepo::AwaitingFirstPushWithMember,
            GitReader::Member,
            TEST_REPO_NAME,
            ViewId::private(),
            StatusCode::NOT_FOUND,
            "repo owner/repo not found",
        ),
    ] {
        let state = test_state_with_repo();
        cache_test_jwks(&state);
        match repo_setup {
            ReadRepo::PrivateOnly => private_only_repo(&state).await,
            ReadRepo::AwaitingFirstPush => await_first_push(&state).await,
            ReadRepo::AwaitingFirstPushWithMember => {
                insert_member(&state, RepositoryMemberPermissions::default()).await;
                await_first_push(&state).await;
            }
        }
        let headers = match reader {
            GitReader::Anonymous => HeaderMap::new(),
            GitReader::Owner => authorization_headers(bearer_header()),
            GitReader::Member => {
                authorization_headers(bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL))
            }
        };

        let error =
            git_upload_pack_repo_for_request(&state, &headers, TEST_REPO_OWNER, repo_name, &view)
                .await
                .unwrap_err();

        assert_eq!(error.status(), status, "{label}");
        assert_eq!(error.public_message(), message, "{label}");
    }
}

#[tokio::test]
async fn receive_pack_accepts_only_the_credential_and_intent_for_the_repo_lifecycle() {
    let invalid = ReceiveOutcome::Rejected(StatusCode::UNAUTHORIZED, "invalid Git credentials");
    for (label, unpublished, credential, push_intent, repo_name, expected) in [
        (
            "wrong ready token",
            false,
            PushCredential::Token("scope_git_wrong"),
            true,
            TEST_REPO_NAME,
            invalid,
        ),
        (
            "ready token for another repo",
            false,
            PushCredential::Token(READY_PUSH_TOKEN),
            true,
            "missing",
            invalid,
        ),
        (
            "first-push token on ready repo",
            false,
            PushCredential::Token("scope_fp_test"),
            true,
            TEST_REPO_NAME,
            invalid,
        ),
        (
            "ready token",
            false,
            PushCredential::Token(READY_PUSH_TOKEN),
            true,
            TEST_REPO_NAME,
            ReceiveOutcome::ReadyMember,
        ),
        (
            "ready token without push intent",
            false,
            PushCredential::Token(READY_PUSH_TOKEN),
            false,
            TEST_REPO_NAME,
            ReceiveOutcome::Rejected(StatusCode::FORBIDDEN, "valid Scope push intent required"),
        ),
        (
            "member session",
            false,
            PushCredential::MemberSession,
            true,
            TEST_REPO_NAME,
            ReceiveOutcome::ReadyMember,
        ),
        (
            "wrong first-push token",
            true,
            PushCredential::Token("scope_fp_wrong"),
            true,
            TEST_REPO_NAME,
            invalid,
        ),
        (
            "ready token on unpublished repo",
            true,
            PushCredential::Token(READY_PUSH_TOKEN),
            true,
            TEST_REPO_NAME,
            invalid,
        ),
        (
            "first-push token",
            true,
            PushCredential::FirstPushSecret,
            true,
            TEST_REPO_NAME,
            ReceiveOutcome::FirstPush,
        ),
    ] {
        let (state, first_push_secret) = if unpublished {
            test_state_with_first_push_token().await
        } else {
            (
                test_state_with_git_push_token(READY_PUSH_TOKEN).await,
                String::new(),
            )
        };
        cache_test_jwks(&state);
        let (mut headers, pusher_id) = match credential {
            PushCredential::Token(secret) => (git_push_token_headers(secret), test_owner_id()),
            PushCredential::FirstPushSecret => {
                (git_push_token_headers(&first_push_secret), test_owner_id())
            }
            PushCredential::MemberSession => (
                authorization_headers(bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL)),
                insert_member(&state, member_permissions(true, false)).await,
            ),
        };
        if push_intent {
            insert_push_intent_header(&state, &mut headers, &pusher_id, TEST_PUSH_HEAD_OID).await;
        }

        match (
            expected,
            receive_pack_access(&state, &headers, TEST_REPO_OWNER, repo_name).await,
        ) {
            (ReceiveOutcome::ReadyMember, Ok(ReceivePackAccess::ReadyMember { author_id, .. })) => {
                assert_eq!(author_id, pusher_id, "{label}");
            }
            (
                ReceiveOutcome::FirstPush,
                Ok(ReceivePackAccess::FirstPush {
                    author_id,
                    incarnation,
                    ..
                }),
            ) => {
                assert_eq!(author_id, test_owner_id(), "{label}");
                assert_eq!(incarnation, test_repo_incarnation(), "{label}");
            }
            (ReceiveOutcome::Rejected(status, message), Err(error)) => {
                assert_eq!(error.status(), status, "{label}");
                assert_eq!(error.public_message(), message, "{label}");
            }
            (_, access) => panic!("{label}: unexpected receive access {access:?}"),
        }
    }
}
