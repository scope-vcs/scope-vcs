use super::*;
use crate::db::joined_rows::{joined, prefixed_columns};
use scope_domain::repository::access::repository_access_for_user_id;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, QueryResult, Statement};

pub(super) struct StateRow(QueryResult);

pub(super) async fn load<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    request_id: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<StateRow>, PostgresError> {
    Ok(conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            state_sql(),
            [request_id.into(), repo_id.into(), viewer_user_id.into()],
        ))
        .await?
        .map(StateRow))
}

impl StateRow {
    pub fn request(&self) -> Result<Request, PostgresError> {
        entities::request::Model::from_query_result(&self.0, "r_")?.try_into_domain()
    }

    pub fn is_invitee(&self) -> Result<bool, PostgresError> {
        Ok(self.0.try_get("", "is_invitee")?)
    }

    pub fn evaluation(&self) -> Result<Option<RequestCheckEvaluation>, PostgresError> {
        if !joined(&self.0, "e_id")? {
            return Ok(None);
        }
        entities::request_check_evaluation::Model::from_query_result(&self.0, "e_")?
            .try_into_domain()
            .map(Some)
    }

    pub fn revision(&self) -> Result<Option<RequestRevision>, PostgresError> {
        if !joined(&self.0, "v_id")? {
            return Ok(None);
        }
        entities::request_revision::Model::from_query_result(&self.0, "v_")?
            .try_into_domain()
            .map(Some)
    }

    pub fn auto_merge(&self) -> Result<Option<RequestAutoMergeIntent>, PostgresError> {
        if !joined(&self.0, "a_id")? {
            return Ok(None);
        }
        entities::request_auto_merge_intent::Model::from_query_result(&self.0, "a_")?
            .try_into_domain()
            .map(Some)
    }

    pub fn auto_merge_actor_access(
        &self,
        repository: &RepositoryAccessContext,
        intent: Option<&RequestAutoMergeIntent>,
    ) -> Result<Option<RepositoryAccess>, PostgresError> {
        let Some(intent) = intent.filter(|intent| intent.is_active()) else {
            return Ok(None);
        };
        let permissions = if joined(&self.0, "m_repo_id")? {
            Some(
                entities::repository_member::Model::from_query_result(&self.0, "m_")?
                    .try_into_domain()?
                    .permissions,
            )
        } else {
            None
        };
        Ok(Some(repository_access_for_user_id(
            &repository.record.owner_user_id,
            repository.record.lifecycle_state,
            permissions,
            &intent.actor_user_id,
        )))
    }

    pub fn users(&self) -> Result<BTreeMap<String, UserAccount>, PostgresError> {
        let mut users = BTreeMap::new();
        for prefix in ["viewer_", "actor_"] {
            if joined(&self.0, &format!("{prefix}id"))? {
                let user =
                    entities::user::Model::from_query_result(&self.0, prefix)?.try_into_domain()?;
                users.insert(user.id.clone(), user);
            }
        }
        Ok(users)
    }

    pub fn canonical_main_oid(&self) -> Result<Option<String>, PostgresError> {
        Ok(self.0.try_get("", "canonical_main_oid")?)
    }
}

fn state_sql() -> &'static str {
    static SQL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SQL.get_or_init(|| {
        format!(
            "SELECT {}, {}, {}, {}, {}, {}, {}, head.head_oid AS canonical_main_oid,
            EXISTS (SELECT 1 FROM scope_request_invitees invitee
                    WHERE invitee.request_id = request.id AND invitee.user_id = $3) AS is_invitee
         FROM scope_requests request
         LEFT JOIN scope_request_check_evaluations evaluation
           ON evaluation.id = request.current_check_evaluation_id
          AND evaluation.request_id = request.id AND evaluation.head_oid = request.head_oid
         LEFT JOIN LATERAL (
             SELECT * FROM scope_request_revisions WHERE request_id = request.id
             ORDER BY position DESC, id DESC LIMIT 1
         ) revision ON TRUE
         LEFT JOIN LATERAL (
             SELECT * FROM scope_request_auto_merge_intents WHERE request_id = request.id
             ORDER BY created_position DESC, created_at_unix DESC, id DESC LIMIT 1
         ) intent ON TRUE
         LEFT JOIN scope_users viewer ON viewer.id = $3
         LEFT JOIN scope_users actor ON actor.id = intent.actor_user_id
         LEFT JOIN scope_repositories repo ON repo.id = request.repo_id
         LEFT JOIN scope_repository_members member ON member.repo_id = request.repo_id
           AND member.user_id = intent.actor_user_id AND member.user_id <> repo.owner_user_id
           AND intent.status = 'Active'
         LEFT JOIN scope_git_heads head ON head.repo_id = request.repo_id
         WHERE request.id = $1 AND request.repo_id = $2",
            prefixed_columns::<entities::request::Column>("request", "r_"),
            prefixed_columns::<entities::request_check_evaluation::Column>("evaluation", "e_"),
            prefixed_columns::<entities::request_revision::Column>("revision", "v_"),
            prefixed_columns::<entities::request_auto_merge_intent::Column>("intent", "a_"),
            prefixed_columns::<entities::user::Column>("viewer", "viewer_"),
            prefixed_columns::<entities::user::Column>("actor", "actor_"),
            prefixed_columns::<entities::repository_member::Column>("member", "m_"),
        )
    })
}
