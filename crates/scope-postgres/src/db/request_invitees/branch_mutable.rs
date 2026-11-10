use super::{RequestStore, entities};
use crate::error::PostgresError;
use scope_domain::requests::{REQUEST_BRANCH_MUTABLE_STATES, RequestState};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, FromQueryResult, QueryFilter, QueryOrder, QuerySelect,
    sea_query::Expr,
};

#[derive(FromQueryResult)]
struct BranchMutableRequestRow {
    #[sea_orm(nested)]
    request: entities::request::Model,
    is_invitee: bool,
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "branch_mutable_requests_with_invitee_status"))]
    pub async fn branch_mutable_requests_with_invitee_status(
        &self,
        repository_id: &str,
        actor_user_id: &str,
    ) -> Result<Vec<(scope_domain::requests::Request, bool)>, PostgresError> {
        let lifecycle = REQUEST_BRANCH_MUTABLE_STATES
            .into_iter()
            .fold(Condition::any(), |condition, state| {
                condition.add(request_state_condition(state))
            });
        entities::request::Entity::find()
            .filter(entities::request::Column::RepoId.eq(repository_id))
            .filter(lifecycle)
            .column_as(
                Expr::cust_with_values(
                    "EXISTS (SELECT 1 FROM scope_request_invitees i
                     WHERE i.request_id = scope_requests.id AND i.user_id = $1)",
                    [actor_user_id],
                ),
                "is_invitee",
            )
            .order_by_asc(entities::request::Column::Id)
            .into_model::<BranchMutableRequestRow>()
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .map(|row| Ok((row.request.try_into_domain()?, row.is_invitee)))
            .collect()
    }
}

fn request_state_condition(state: RequestState) -> Condition {
    use entities::request::Column;
    match state {
        RequestState::Draft | RequestState::Open => Condition::all()
            .add(Column::MergedAtUnix.is_null())
            .add(Column::ClosedAtUnix.is_null())
            .add(if state == RequestState::Draft {
                Column::SubmittedAtUnix.is_null()
            } else {
                Column::SubmittedAtUnix.is_not_null()
            }),
        RequestState::Closed => Condition::all()
            .add(Column::MergedAtUnix.is_null())
            .add(Column::ClosedAtUnix.is_not_null()),
        RequestState::Merged => Condition::all().add(Column::MergedAtUnix.is_not_null()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{postgres_store, start_public_request};
    use crate::db::AddRequestInviteeCommand;

    #[tokio::test]
    async fn push_candidates_exclude_terminal_history_and_preserve_invitee_status() {
        let store = postgres_store(1).await;
        for id in ["draft", "open", "closed", "merged"] {
            start_public_request(&store, id).await;
            if id != "draft" {
                store.admin().execute_for_tests(&format!(
                    "UPDATE scope_requests SET submitted_at_unix = 3, updated_at_unix = 4 WHERE id = '{id}'"
                )).await.unwrap();
            }
        }
        store
            .admin()
            .execute_for_tests(
                "UPDATE scope_requests SET closed_at_unix = 4 WHERE id = 'closed';
             UPDATE scope_requests SET merged_at_unix = 4, merged_head_oid = head_oid,
                 merged_main_oid = head_oid WHERE id = 'merged'",
            )
            .await
            .unwrap();
        store
            .requests()
            .add_request_invitee(AddRequestInviteeCommand {
                request_id: "draft".into(),
                actor_user_id: "user_author".into(),
                target_handle: "target-0".into(),
                now_unix: 3,
            })
            .await
            .unwrap();
        let requests = store.requests();
        let candidates = requests
            .branch_mutable_requests_with_invitee_status("owner/repo", "user_target_0")
            .await
            .unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|(request, invited)| (request.id.as_str(), *invited))
                .collect::<Vec<_>>(),
            [("draft", true), ("open", false)]
        );
        let authored = requests
            .branch_mutable_requests_with_invitee_status("owner/repo", "user_author")
            .await
            .unwrap();
        assert!(authored.iter().all(|(_, invited)| !invited));
        assert_eq!(
            requests
                .requests_by_repo_id("owner/repo")
                .await
                .unwrap()
                .len(),
            4
        );
        assert!(
            requests
                .branch_mutable_requests_with_invitee_status("missing/repo", "user_target_0")
                .await
                .unwrap()
                .is_empty()
        );
    }
}
