use super::*;
use crate::db::{
    MetadataStore,
    generated_ids::test_generated_id,
    test_support::fixtures::{repository, store_with_repositories, user},
};
use crate::error::PostgresErrorKind;
use scope_domain::{
    account::{ExternalIdentity, deletion::CLERK_USER_DELETION_TOMBSTONE_SECS},
    policy::Visibility,
    repository::collaboration::RepositoryMember,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde_json::{Value, json};

fn member(repo: &str, user_id: &str) -> RepositoryMember {
    RepositoryMember {
        repo_id: repo.into(),
        user_id: user_id.into(),
        permissions: Default::default(),
        created_at_unix: 1,
        updated_at_unix: 1,
    }
}

async fn query_json(store: &MetadataStore, sql: &str) -> Value {
    let row = store
        .db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            format!("SELECT to_jsonb(({sql})) AS value"),
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<Value>("", "value").unwrap()
}

// The leaver joined owner/shared through an invite and contributed as a
// member: authored, closed, merged, rated, invited, reviewed, requested a run
// and authorized an auto-merge. They also own a solo repository.
const CONTRIBUTIONS: &str = r#"
    INSERT INTO scope_auth_identities VALUES ('clerk', 'user_clerk_leaver', 'leaver');
    INSERT INTO scope_cli_sessions (id, token_hash, user_id, label, created_at_unix, expires_at_unix)
        VALUES ('cli', repeat('f', 64), 'leaver', 'laptop', 1, 999);
    INSERT INTO scope_requests (id, repo_id, name, author_user_id, author_role, audience,
        base_main_oid, head_oid, title, description_markdown, activity_version,
        submitted_at_unix, closed_at_unix, closed_by_user_id, merged_at_unix,
        merged_by_user_id, merged_head_oid, merged_main_oid, created_at_unix, updated_at_unix)
    VALUES
        ('authored', 'owner/shared', 'authored', 'leaver', 'Member', 'Private', repeat('a', 40),
            repeat('b', 40), 'Authored', '', 2, 2, 3, 'leaver', NULL, NULL, NULL, NULL, 1, 3),
        ('merged', 'owner/shared', 'merged', 'owner', 'Owner', 'Private', repeat('a', 40),
            repeat('b', 40), 'Merged', '', 2, 2, NULL, NULL, 3, 'leaver', repeat('b', 40),
            repeat('c', 40), 1, 3),
        ('open', 'owner/shared', 'open', 'owner', 'Owner', 'Private', repeat('a', 40),
            repeat('b', 40), 'Open', '', 2, 2, NULL, NULL, NULL, NULL, NULL, NULL, 1, 3);
    INSERT INTO scope_request_events (id, request_id, actor_user_id, kind, position, payload,
        created_at_unix)
        VALUES ('event', 'authored', 'leaver', 'Closed',
            2, jsonb_build_object('Closed', jsonb_build_object('head_oid', repeat('b', 40))), 3);
    INSERT INTO scope_request_revisions (id, request_id, position, actor_user_id, old_head_oid,
        new_head_oid, git_snapshot, created_at_unix)
        VALUES ('revision', 'open', 1, 'leaver', repeat('a', 40), repeat('b', 40), '{}', 2);
    INSERT INTO scope_request_discussions (id, request_id, opened_position, last_activity_position,
        author_user_id, body_markdown, status, client_discussion_id, created_at_unix)
        VALUES ('discussion', 'merged', 1, 2, 'leaver', 'Looks good', 'Open', 'client', 2);
    INSERT INTO scope_request_discussion_replies (id, discussion_id, position, author_user_id,
        body_markdown, client_reply_id, created_at_unix)
        VALUES ('reply', 'discussion', 2, 'leaver', 'Thanks', 'client', 3);
    INSERT INTO scope_request_ratings (id, request_id, rater_user_id, subject_user_id, score,
        reason, created_at_unix)
        VALUES ('rating', 'merged', 'leaver', 'owner', 5, 'Clear', 4);
    INSERT INTO scope_request_invitees VALUES ('authored', 'guest', 'leaver', 2);
    INSERT INTO scope_request_claims VALUES ('open', 'leaver', 2, 2);
    INSERT INTO scope_repository_invites (id, repo_id, invited_email, invited_email_normalized,
        permissions, invited_by_user_id, created_at_unix, updated_at_unix, expires_at_unix,
        accepted_by_user_id, accepted_at_unix)
        VALUES ('invite', 'owner/shared', 'Leaver@scope.test', 'leaver@scope.test',
            '{"can_push": false, "can_change_file_visibility": false}', 'owner', 1, 1, 999,
            'leaver', 2);
    INSERT INTO scope_request_auto_merge_intents (id, repo_id, repository_incarnation_id,
        request_id, revision_id, head_oid, actor_user_id, status, created_position,
        next_attempt_at_unix, created_at_unix, updated_at_unix)
        VALUES ('intent', 'owner/shared', 'repoi_owner_shared', 'open', 'revision',
            repeat('b', 40), 'leaver', 'Active', 2, 3, 3, 3);
    INSERT INTO scope_workflow_revisions VALUES (repeat('d', 64), '{"jobs":[{"id":"build"}]}', 1);
    INSERT INTO scope_runs (id, idempotency_key, repo_id, workflow_path, workflow_revision_digest,
        trigger, requested_by_user_id, source, state, cancellation_requested, created_at_unix,
        updated_at_unix)
        VALUES ('run', 'key', 'owner/shared', '/.scope/runs/checks.yml', repeat('d', 64),
            'manual', 'leaver', jsonb_build_object('kind', 'ephemeral-git-bundle', 'object',
            jsonb_build_object('sha256', repeat('e', 64), 'git_oid', repeat('b', 40))),
            'queued', false, 3, 3);
"#;

#[tokio::test]
async fn deleting_an_account_keeps_its_work_in_other_repositories() {
    let owner = user("owner", "owner");
    let leaver = user("leaver", "leaver");
    let mut shared = repository(&owner, "shared", Visibility::Private);
    shared.members = vec![
        member("owner/shared", "leaver"),
        member("owner/shared", "guest"),
    ];
    let shared_version = shared.record.change_version;
    let store = store_with_repositories([shared, repository(&leaver, "solo", Visibility::Private)]);
    store.db.execute_unprepared(CONTRIBUTIONS).await.unwrap();

    let deleted = store
        .auth()
        .delete_account("leaver", 10, &test_generated_id)
        .await
        .unwrap();

    assert_eq!(deleted.deleted_repositories.len(), 1);
    assert_eq!(
        deleted.deleted_repositories[0].incarnation.repository_id(),
        "leaver/solo"
    );
    assert_eq!(deleted.changed_repositories.len(), 1);
    assert_eq!(
        deleted.changed_repositories[0].change_version,
        shared_version + 1
    );
    // The shared repository is already announced as changed.
    assert!(deleted.contributed_repositories.is_empty());
    let survivors = query_json(
        &store,
        r#"jsonb_build_object(
            'requests', (SELECT jsonb_agg(jsonb_build_array(id, author_user_id, closed_by_user_id,
                merged_by_user_id) ORDER BY id) FROM scope_requests),
            'event', (SELECT actor_user_id FROM scope_request_events WHERE id = 'event'),
            'stopped', (SELECT jsonb_build_array(kind, actor_user_id) FROM scope_request_events
                WHERE kind = 'AutoMergeStopped'),
            'revision', (SELECT actor_user_id FROM scope_request_revisions),
            'discussion', (SELECT author_user_id FROM scope_request_discussions),
            'reply', (SELECT author_user_id FROM scope_request_discussion_replies),
            'invitee', (SELECT jsonb_build_array(user_id, invited_by_user_id)
                FROM scope_request_invitees),
            'run', (SELECT requested_by_user_id FROM scope_runs),
            'members', (SELECT jsonb_agg(user_id) FROM scope_repository_members),
            'repositories', (SELECT jsonb_agg(id) FROM scope_repositories),
            'personal', (SELECT count(*) FROM scope_request_ratings)
                + (SELECT count(*) FROM scope_request_claims)
                + (SELECT count(*) FROM scope_request_auto_merge_intents)
                + (SELECT count(*) FROM scope_repository_invites)
                + (SELECT count(*) FROM scope_auth_identities)
                + (SELECT count(*) FROM scope_cli_sessions)
                + (SELECT count(*) FROM scope_users WHERE id = 'leaver'),
            'clerk', (SELECT jsonb_agg(clerk_user_id) FROM scope_clerk_user_deletions)
        )"#,
    )
    .await;
    assert_eq!(
        survivors,
        json!({
            "requests": [
                ["authored", null, null, null],
                ["merged", "owner", null, null],
                ["open", "owner", null, null],
            ],
            "event": null,
            "stopped": ["AutoMergeStopped", null],
            "revision": null,
            "discussion": null,
            "reply": null,
            "invitee": ["guest", null],
            "run": null,
            "members": ["guest"],
            "repositories": ["owner/shared"],
            "personal": 0,
            "clerk": ["user_clerk_leaver"],
        })
    );

    // The survivors still read as domain facts.
    let merged = store
        .requests()
        .request_for_tests("merged")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(merged.merged_by_user_id, None);
    assert!(merged.merged_at_unix.is_some());
    let cleanups = store
        .cleanup()
        .pending_repo_storage_cleanups_for_tests()
        .await
        .unwrap();
    assert_eq!(cleanups.len(), 1);
    assert_eq!(cleanups[0].repo_name, "solo");

    let leftover_email = query_json(
        &store,
        r#"(SELECT coalesce(jsonb_agg(c.relname), '[]') FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = current_schema() AND c.relkind = 'r'
              AND (xpath('/row/c/text()', query_to_xml(format(
                  'SELECT count(*) AS c FROM %I t WHERE t::text ILIKE %L',
                  c.relname, '%leaver@scope.test%'), false, true, '')))[1]::text::int > 0)"#,
    )
    .await;
    assert_eq!(leftover_email, json!([]));

    // Signing back in would create an account the pending Clerk deletion
    // then strands.
    let refused = store
        .auth()
        .resolve_clerk_user(
            &ExternalIdentity {
                provider: "clerk".into(),
                subject: "user_clerk_leaver".into(),
                email: Some(leaver.email.clone()),
                email_verified: true,
            },
            11,
        )
        .await
        .unwrap_err();
    assert_eq!(refused.kind, PostgresErrorKind::Unauthenticated);
}

#[tokio::test]
async fn deleting_an_account_removes_its_drafts_and_announces_its_contributions() {
    let owner = user("owner", "owner");
    let leaver = user("leaver", "leaver");
    let store = store_with_repositories([
        repository(&owner, "public", Visibility::Public),
        repository(&leaver, "solo", Visibility::Private),
    ]);
    // Without membership, the leaver submitted one public request and left
    // another as a draft, which nobody else could delete once they are gone.
    store
        .db
        .execute_unprepared(
            "INSERT INTO scope_requests (id, repo_id, name, author_user_id, author_role, audience,
                base_main_oid, head_oid, title, description_markdown, activity_version,
                submitted_at_unix, created_at_unix, updated_at_unix)
            VALUES
                ('submitted', 'owner/public', 'submitted', 'leaver', 'Public', 'Public',
                    repeat('a', 40), repeat('b', 40), 'Submitted', '', 1, 2, 1, 2),
                ('draft', 'owner/public', 'draft', 'leaver', 'Public', 'Public',
                    repeat('a', 40), repeat('b', 40), 'Draft', '', 1, NULL, 1, 1)",
        )
        .await
        .unwrap();

    let deleted = store
        .auth()
        .delete_account("leaver", 10, &test_generated_id)
        .await
        .unwrap();

    assert!(deleted.changed_repositories.is_empty());
    assert_eq!(
        deleted
            .contributed_repositories
            .iter()
            .map(|incarnation| incarnation.repository_id())
            .collect::<Vec<_>>(),
        ["owner/public"]
    );
    assert_eq!(
        query_json(
            &store,
            "SELECT jsonb_agg(jsonb_build_array(id, author_user_id)) FROM scope_requests"
        )
        .await,
        json!([["submitted", null]])
    );
}

#[tokio::test]
async fn owning_a_repository_with_another_member_blocks_deletion() {
    let leaver = user("leaver", "leaver");
    let mut team = repository(&leaver, "team", Visibility::Private);
    team.members = vec![member("leaver/team", "friend")];
    let store = store_with_repositories([team, repository(&leaver, "solo", Visibility::Private)]);

    let refused = store
        .auth()
        .delete_account("leaver", 10, &test_generated_id)
        .await
        .unwrap_err();

    let AccountDeletionError::SharedRepositories(shared) = refused else {
        panic!("expected a refusal, got {refused:?}");
    };
    assert_eq!(shared.repository_ids, ["leaver/team"]);
    let remaining = query_json(
        &store,
        "SELECT jsonb_build_array((SELECT count(*) FROM scope_users WHERE id = 'leaver'),
            (SELECT count(*) FROM scope_repositories))",
    )
    .await;
    assert_eq!(remaining, json!([1, 2]));
}

#[tokio::test]
async fn a_failed_clerk_deletion_waits_and_is_claimed_again() {
    let store = store_with_repositories([]);
    store
        .db
        .execute_unprepared(
            "INSERT INTO scope_clerk_user_deletions (clerk_user_id, next_attempt_at_unix,
                created_at_unix) VALUES ('user_clerk', 10, 10)",
        )
        .await
        .unwrap();
    let auth = store.auth();

    assert_eq!(
        auth.claim_due_clerk_user_deletions("first", 10, 130, 5)
            .await
            .unwrap(),
        ["user_clerk"]
    );
    // A second worker does not take a claimed deletion.
    assert!(
        auth.claim_due_clerk_user_deletions("second", 11, 131, 5)
            .await
            .unwrap()
            .is_empty()
    );
    auth.retry_clerk_user_deletion("user_clerk", "first", "Clerk answered 503", 12)
        .await
        .unwrap();
    assert!(
        auth.claim_due_clerk_user_deletions("second", 41, 161, 5)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        auth.claim_due_clerk_user_deletions("second", 42, 162, 5)
            .await
            .unwrap(),
        ["user_clerk"]
    );
    // The first worker's lapsed claim no longer settles the deletion.
    auth.complete_clerk_user_deletion("user_clerk", "first", 43)
        .await
        .unwrap();
    auth.complete_clerk_user_deletion("user_clerk", "second", 43)
        .await
        .unwrap();
    let completed = "SELECT jsonb_agg(completed_at_unix) FROM scope_clerk_user_deletions";
    assert_eq!(query_json(&store, completed).await, json!([43]));
    // A completed deletion is never claimed again, and stays until tokens
    // issued before it have expired.
    assert!(
        auth.claim_due_clerk_user_deletions("third", 10_000, 10_120, 5)
            .await
            .unwrap()
            .is_empty()
    );
    let tombstone_ends = 43 + CLERK_USER_DELETION_TOMBSTONE_SECS;
    auth.purge_completed_clerk_user_deletions(tombstone_ends - 1)
        .await
        .unwrap();
    assert_eq!(query_json(&store, completed).await, json!([43]));
    auth.purge_completed_clerk_user_deletions(tombstone_ends)
        .await
        .unwrap();
    assert_eq!(
        query_json(&store, "SELECT count(*) FROM scope_clerk_user_deletions").await,
        json!(0)
    );
}
