use super::{
    auth::{i64_to_u64, load_user_by_id, u64_to_i64},
    cli_auth_results::{CliSessionSummary, NewCliSession},
    entities,
};
use crate::error::PostgresError;
use scope_domain::account::SessionIdentity;
use sea_orm::{ActiveModelTrait, ConnectionTrait, DatabaseBackend, IntoActiveModel, Statement};

pub async fn insert_cli_session_in_tx<C>(
    conn: &C,
    user_id: &str,
    session: NewCliSession,
) -> Result<SessionIdentity, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    entities::cli_session::Model {
        id: session.id,
        token_hash: session.token_hash,
        user_id: user_id.to_string(),
        label: session.label,
        created_at_unix: u64_to_i64(session.created_at_unix)?,
        last_used_at_unix: None,
        expires_at_unix: u64_to_i64(session.expires_at_unix)?,
        revoked_at_unix: None,
    }
    .into_active_model()
    .insert(conn)
    .await
    .map_err(PostgresError::internal)?;
    let user = load_user_by_id(conn, user_id).await?;
    Ok(SessionIdentity::from(&user))
}

pub fn cli_session_summary_from_model(
    session: entities::cli_session::Model,
) -> Result<CliSessionSummary, PostgresError> {
    Ok(CliSessionSummary {
        id: session.id,
        label: session.label,
        created_at_unix: i64_to_u64(session.created_at_unix)?,
        last_used_at_unix: session.last_used_at_unix.map(i64_to_u64).transpose()?,
        expires_at_unix: i64_to_u64(session.expires_at_unix)?,
    })
}

// Session activity is approximate to one minute; successful reads do not write on every request.
const CLI_SESSION_ACTIVITY_INTERVAL_SECONDS: u64 = 60;

pub(super) async fn record_cli_session_use<C: ConnectionTrait>(
    conn: &C,
    session_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_cli_sessions SET last_used_at_unix = $2
         WHERE id = $1 AND revoked_at_unix IS NULL AND expires_at_unix > $2
           AND (last_used_at_unix IS NULL OR last_used_at_unix <= $3)",
        [
            session_id.into(),
            u64_to_i64(now_unix)?.into(),
            u64_to_i64(now_unix.saturating_sub(CLI_SESSION_ACTIVITY_INTERVAL_SECONDS))?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::fixtures::{store_with_repositories, user};
    use sea_orm::EntityTrait;

    #[tokio::test]
    async fn successful_session_authentication_records_throttled_monotonic_activity() {
        let store = store_with_repositories([]);
        let user = user("user_session_activity", "session-activity");
        store
            .auth()
            .insert_user_for_tests(user.clone())
            .await
            .unwrap();
        insert_cli_session_in_tx(
            store.db.as_ref(),
            &user.id,
            NewCliSession {
                id: "session_activity".into(),
                token_hash: "activity-token-hash".into(),
                label: "Activity test".into(),
                created_at_unix: 100,
                expires_at_unix: 1000,
            },
        )
        .await
        .unwrap();
        let auth = store.auth();
        for (now, expected) in [(110, 110), (120, 110), (170, 170), (160, 170)] {
            auth.verify_cli_session_by_hash("activity-token-hash", now)
                .await
                .unwrap();
            let session = entities::cli_session::Entity::find_by_id("session_activity")
                .one(store.db.as_ref())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(session.last_used_at_unix, Some(expected));
        }
        assert!(
            auth.verify_cli_session_by_hash("activity-token-hash", 1000)
                .await
                .is_err()
        );
        auth.revoke_cli_session_by_hash("activity-token-hash", 180)
            .await
            .unwrap();
        assert!(
            auth.verify_cli_session_by_hash("activity-token-hash", 190)
                .await
                .is_err()
        );
        let session = entities::cli_session::Entity::find_by_id("session_activity")
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.last_used_at_unix, Some(170));
    }
}
