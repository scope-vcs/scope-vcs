use super::*;

const SEED: &str = r#"
    INSERT INTO scope_users VALUES ('owner','owner','owner@scope.test',true);
    INSERT INTO scope_repositories (id,owner_handle,name,owner_user_id,publication_state,
        change_version,repo_config,policy,incarnation_id)
        VALUES ('owner/repo','owner','repo','owner','Ready',0,'{}','{}','repoi_m0057');
    INSERT INTO scope_repository_invites (id,repo_id,invited_email,invited_email_normalized,
        permissions,invited_by_user_id,state,token_hash,created_at_unix,updated_at_unix,
        expires_at_unix,accepted_by_user_id,accepted_at_unix,revoked_at_unix)
        VALUES
        ('pending','owner/repo','a@scope.test','a@scope.test','{}','owner','Pending','hash-pending',10,10,900,NULL,NULL,NULL),
        ('expired','owner/repo','b@scope.test','b@scope.test','{}','owner','Expired','hash-expired',10,50,900,NULL,NULL,NULL),
        ('revoked','owner/repo','c@scope.test','c@scope.test','{}','owner','Revoked','hash-revoked',10,60,900,NULL,NULL,60),
        ('accepted','owner/repo','d@scope.test','d@scope.test','{}','owner','Accepted','hash-accepted',10,70,900,'owner',70,NULL);
"#;

#[tokio::test]
async fn existing_invites_keep_their_link_and_their_state() {
    let (_target, db, _lease) = isolated_database().await;
    let before_invite_links = LATEST_MIGRATIONS
        .iter()
        .position(|name| *name == "m0057_repository_invite_links")
        .and_then(|index| u32::try_from(index).ok())
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(before_invite_links))
        .await
        .unwrap();
    db.execute_unprepared(SEED).await.unwrap();

    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();

    let rows = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "
            SELECT invite.id, link.token_hash, invite.expires_at_unix,
                   invite.accepted_at_unix, invite.revoked_at_unix
            FROM scope_repository_invites invite
            JOIN scope_repository_invite_links link ON link.invite_id = invite.id
            ORDER BY invite.id
            "
            .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "id").unwrap(),
                row.try_get::<String>("", "token_hash").unwrap(),
                row.try_get::<i64>("", "expires_at_unix").unwrap(),
                row.try_get::<Option<i64>>("", "accepted_at_unix").unwrap(),
                row.try_get::<Option<i64>>("", "revoked_at_unix").unwrap(),
            )
        })
        .collect::<Vec<_>>();

    // The stored state is gone, so the timestamps alone must still say
    // accepted, expired, pending, and revoked.
    assert_eq!(
        rows,
        [
            (
                "accepted".into(),
                "hash-accepted".into(),
                900,
                Some(70),
                None
            ),
            ("expired".into(), "hash-expired".into(), 50, None, None),
            ("pending".into(), "hash-pending".into(), 900, None, None),
            ("revoked".into(), "hash-revoked".into(), 900, None, Some(60)),
        ]
    );
}
