use super::*;

#[tokio::test]
async fn deleting_a_repository_keeps_its_emails_for_the_owner_allowance() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users VALUES ('owner','owner','owner@scope.test',true);
        INSERT INTO scope_repositories (id,owner_handle,name,owner_user_id,publication_state,
            change_version,repo_config,policy,incarnation_id)
            VALUES ('owner/repo','owner','repo','owner','Ready',0,'{}','{}','repoi_m0058');
        INSERT INTO scope_repository_invites (id,repo_id,invited_email,invited_email_normalized,
            permissions,invited_by_user_id,created_at_unix,updated_at_unix,expires_at_unix)
            VALUES ('invite','owner/repo','a@scope.test','a@scope.test','{}','owner',10,10,900);
        INSERT INTO scope_repository_invite_emails (id,invite_id,requested_by_user_id,state,
            next_attempt_at_unix,created_at_unix,updated_at_unix)
            VALUES ('email','invite','owner','Sent',10,10,10);
        DELETE FROM scope_repositories WHERE id = 'owner/repo';
        "#,
    )
    .await
    .unwrap();

    let row = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT invite_id, requested_by_user_id FROM scope_repository_invite_emails"
                .to_string(),
        ))
        .await
        .unwrap()
        .expect("the email outlives its repository");
    assert_eq!(
        row.try_get::<Option<String>>("", "invite_id").unwrap(),
        None
    );
    assert_eq!(
        row.try_get::<String>("", "requested_by_user_id").unwrap(),
        "owner"
    );
    // Only a queued email may be claimed.
    assert!(
        db.execute_unprepared(
            "UPDATE scope_repository_invite_emails
             SET claim_token = 'claim', claim_expires_at_unix = 20"
        )
        .await
        .is_err()
    );
}
