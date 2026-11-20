use super::{
    RequestStore, entities,
    integer_columns::i64_to_u64,
    request_access::{ensure_user_exists, lock_request_repository},
};
use crate::error::PostgresError;
use scope_domain::requests::{
    AUTHOR_RECORD_ROLE, AuthorRecordTotals, CreateRequestRatingInput, RequestRating,
    create_request_rating,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, Statement, TransactionTrait,
};
use std::collections::HashMap;

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_ratings"))]
    pub async fn request_ratings(
        &self,
        request_id: &str,
    ) -> Result<Vec<RequestRating>, PostgresError> {
        ratings_for_request(self.db.as_ref(), request_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "create_request_rating"))]
    pub async fn create_request_rating(
        &self,
        input: CreateRequestRatingInput,
    ) -> Result<RequestRating, PostgresError> {
        let tx = self.db.begin().await?;
        let (_repo, request) =
            lock_request_repository(&tx, &input.request_id, &input.actor_user_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let ratings = ratings_for_request(&tx, &request.id).await?;
        let rating = create_request_rating(&request, &ratings, input)?;
        entities::request_rating::Model::from_domain(&rating)?
            .into_active_model()
            .insert(&tx)
            .await?;
        tx.commit().await?;
        Ok(rating)
    }
}

#[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "author_record_totals"))]
pub(super) async fn author_record_totals<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    author_user_ids: Vec<String>,
) -> Result<HashMap<String, AuthorRecordTotals>, PostgresError> {
    let rows = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
                SELECT requests.author_user_id,
                       COUNT(*) FILTER (
                           WHERE requests.merged_at_unix IS NOT NULL
                       ) AS merged,
                       COUNT(*) FILTER (
                           WHERE requests.closed_at_unix IS NOT NULL
                             AND requests.closed_by_user_id
                                 IS DISTINCT FROM requests.author_user_id
                       ) AS closed_by_maintainer,
                       COUNT(*) FILTER (
                           WHERE requests.closed_at_unix IS NOT NULL
                             AND requests.closed_by_user_id = requests.author_user_id
                       ) AS withdrawn,
                       COUNT(*) FILTER (
                           WHERE requests.merged_at_unix IS NULL
                             AND requests.closed_at_unix IS NULL
                       ) AS open,
                       COALESCE(SUM(ratings.score), 0) AS rating_score_sum,
                       COUNT(ratings.id) AS rating_count
                FROM scope_requests requests
                LEFT JOIN scope_request_ratings ratings
                       ON ratings.request_id = requests.id
                      AND ratings.subject_user_id = requests.author_user_id
                WHERE requests.repo_id = $1
                  AND requests.author_user_id = ANY($2)
                  AND requests.author_role = $3
                  AND requests.submitted_at_unix IS NOT NULL
                GROUP BY requests.author_user_id
            "#,
            [
                repo_id.into(),
                author_user_ids.into(),
                entities::encode_enum(AUTHOR_RECORD_ROLE)?.into(),
            ],
        ))
        .await?;
    rows.into_iter()
        .map(|row| {
            let count = |column: &str| i64_to_u64(row.try_get("", column)?, column);
            Ok((
                row.try_get::<String>("", "author_user_id")?,
                AuthorRecordTotals {
                    merged: count("merged")?,
                    closed_by_maintainer: count("closed_by_maintainer")?,
                    withdrawn: count("withdrawn")?,
                    open: count("open")?,
                    rating_score_sum: count("rating_score_sum")?,
                    rating_count: count("rating_count")?,
                },
            ))
        })
        .collect()
}

async fn ratings_for_request<C>(
    conn: &C,
    request_id: &str,
) -> Result<Vec<RequestRating>, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    entities::request_rating::Entity::find()
        .filter(entities::request_rating::Column::RequestId.eq(request_id))
        .order_by_asc(entities::request_rating::Column::CreatedAtUnix)
        .order_by_asc(entities::request_rating::Column::Id)
        .all(conn)
        .await?
        .into_iter()
        .map(entities::request_rating::Model::try_into_domain)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::requests::tests::{postgres_store, start_public_request};
    use scope_domain::requests::{Request, RequestActorRole, StoredRequest};

    #[tokio::test]
    async fn terminal_participants_persist_at_most_one_immutable_rating_each() {
        let store = postgres_store();
        start_public_request(&store).await;
        crate::db::test_support::request_fixtures::submitted(
            &store.requests(),
            scope_domain::requests::SubmitRequestInput {
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                event_id: "event_submitted".to_string(),
                now_unix: 4,
            },
        )
        .await;
        crate::db::test_support::request_fixtures::closed(
            &store.requests(),
            scope_domain::requests::CloseRequestInput {
                actor_is_maintainer: true,
                request_id: "req_1".to_string(),
                actor_user_id: "user_owner".to_string(),
                event_id: "event_closed".to_string(),
                now_unix: 5,
            },
        )
        .await;

        store
            .requests()
            .create_request_rating(CreateRequestRatingInput {
                id: "rating_author".to_string(),
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                score: 5,
                reason: "Thoughtful review".to_string(),
                now_unix: 6,
            })
            .await
            .unwrap();
        assert!(
            store
                .requests()
                .create_request_rating(CreateRequestRatingInput {
                    id: "rating_author_again".to_string(),
                    request_id: "req_1".to_string(),
                    actor_user_id: "user_public".to_string(),
                    score: 4,
                    reason: "Again".to_string(),
                    now_unix: 7,
                })
                .await
                .is_err()
        );
        store
            .requests()
            .create_request_rating(CreateRequestRatingInput {
                id: "rating_maintainer".to_string(),
                request_id: "req_1".to_string(),
                actor_user_id: "user_owner".to_string(),
                score: 4,
                reason: "Useful contribution".to_string(),
                now_unix: 8,
            })
            .await
            .unwrap();
        assert_eq!(
            store
                .requests()
                .request_ratings("req_1")
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn author_totals_count_a_contributors_submitted_requests_in_one_repository() {
        let store = postgres_store();
        start_public_request(&store).await;
        let requests = store.requests();
        crate::db::test_support::request_fixtures::submitted(
            &requests,
            scope_domain::requests::SubmitRequestInput {
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                event_id: "event_submitted".to_string(),
                now_unix: 4,
            },
        )
        .await;
        let open = requests.request_by_id("req_1").await.unwrap().unwrap();
        let merge = |stored: &mut StoredRequest| {
            stored.lifecycle.merged_at_unix = Some(5);
            stored.lifecycle.merged_by_user_id = Some("user_owner".to_string());
            stored.lifecycle.merged_head_oid = Some(stored.head_oid.clone());
            stored.lifecycle.merged_main_oid = Some(stored.head_oid.clone());
        };
        let close = |stored: &mut StoredRequest, closer: &str| {
            stored.lifecycle.closed_at_unix = Some(5);
            stored.lifecycle.closed_by_user_id = Some(closer.to_string());
        };
        let variant = |id: &str, edit: &dyn Fn(&mut StoredRequest)| {
            let mut stored = open.stored();
            stored.id = id.to_string();
            stored.name = id.replace('_', "-");
            edit(&mut stored);
            stored.updated_at_unix = 5;
            Request::restore(stored).unwrap()
        };
        for request in [
            variant("req_merged", &merge),
            variant("req_closed", &|stored| close(stored, "user_owner")),
            variant("req_closed_deleted_maintainer", &|stored| {
                stored.lifecycle.closed_at_unix = Some(5);
            }),
            variant("req_withdrawn", &|stored| close(stored, "user_public")),
            variant("req_draft", &|stored| {
                stored.lifecycle.submitted_at_unix = None
            }),
            variant("req_as_owner", &|stored| {
                stored.author_role = RequestActorRole::Owner;
                merge(stored);
            }),
            variant("req_other_author", &|stored| {
                stored.author_user_id = Some("user_guest".to_string());
            }),
        ] {
            entities::request::Model::from_domain(&request)
                .unwrap()
                .into_active_model()
                .insert(requests.db.as_ref())
                .await
                .unwrap();
        }
        for (id, request_id, rater, subject, score) in [
            (
                "rating_of_author",
                "req_closed",
                "user_owner",
                "user_public",
                4,
            ),
            (
                "rating_of_maintainer",
                "req_closed",
                "user_public",
                "user_owner",
                1,
            ),
            (
                "rating_as_owner",
                "req_as_owner",
                "user_owner",
                "user_public",
                1,
            ),
        ] {
            entities::request_rating::Model::from_domain(&RequestRating {
                id: id.to_string(),
                request_id: request_id.to_string(),
                rater_user_id: rater.to_string(),
                subject_user_id: subject.to_string(),
                score,
                reason: "Reason".to_string(),
                created_at_unix: 6,
            })
            .unwrap()
            .into_active_model()
            .insert(requests.db.as_ref())
            .await
            .unwrap();
        }

        let authors = vec!["user_public".to_string(), "user_owner".to_string()];
        assert_eq!(
            author_record_totals(requests.db.as_ref(), &open.repo_id, authors.clone())
                .await
                .unwrap(),
            HashMap::from([(
                "user_public".to_string(),
                AuthorRecordTotals {
                    merged: 1,
                    closed_by_maintainer: 2,
                    withdrawn: 1,
                    open: 1,
                    rating_score_sum: 4,
                    rating_count: 1,
                },
            )])
        );
        assert_eq!(
            author_record_totals(requests.db.as_ref(), "another_repository", authors)
                .await
                .unwrap(),
            HashMap::new()
        );
    }
}
