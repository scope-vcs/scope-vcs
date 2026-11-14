use super::{RequestStore, entities};
use crate::error::PostgresError;
use scope_domain::requests::{GitFetchCandidates, Request};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, FromQueryResult, QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, SimpleExpr},
};

#[derive(FromQueryResult)]
struct GitFetchRequestRow {
    #[sea_orm(nested)]
    request: entities::request::Model,
    is_invitee: bool,
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "git_fetch_requests"))]
    pub async fn git_fetch_requests(
        &self,
        repository_id: &str,
        viewer_user_id: Option<&str>,
        candidates: &GitFetchCandidates,
        advertised_request_ids: &[String],
    ) -> Result<Vec<(Request, bool)>, PostgresError> {
        use entities::request::Column;
        let invitee = match viewer_user_id {
            Some(user) => Expr::cust_with_values(
                "EXISTS (SELECT 1 FROM scope_request_invitees i
                 WHERE i.request_id = scope_requests.id AND i.user_id = $1)",
                [user],
            ),
            None => Expr::val(false),
        };
        let own = match viewer_user_id {
            Some(user) => Condition::any()
                .add(Column::AuthorUserId.eq(user))
                .add(invitee.clone()),
            None => Condition::all().add(Expr::val(false)),
        };
        let selected = match candidates {
            GitFetchCandidates::Every => Condition::all(),
            GitFetchCandidates::Listing {
                own_name_prefixes,
                names,
            } => Condition::any()
                .add(Column::Name.is_in(names.iter().cloned()))
                .add(match own_name_prefixes {
                    None => own,
                    Some(prefixes) => Condition::all().add(own).add(name_has_any_prefix(prefixes)),
                }),
            GitFetchCandidates::Heads(heads) => {
                Condition::any().add(Column::HeadOid.is_in(heads.iter().cloned()))
            }
        };
        entities::request::Entity::find()
            .filter(Column::RepoId.eq(repository_id))
            .filter(Column::MergedAtUnix.is_null())
            .filter(Column::ClosedAtUnix.is_null())
            .filter(
                Condition::any()
                    .add(selected)
                    .add(Column::Id.is_in(advertised_request_ids.iter().cloned())),
            )
            .column_as(invitee, "is_invitee")
            .order_by_asc(Column::Name)
            .into_model::<GitFetchRequestRow>()
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .map(|row| Ok((row.request.try_into_domain()?, row.is_invitee)))
            .collect()
    }
}

fn name_has_any_prefix(prefixes: &std::collections::BTreeSet<String>) -> SimpleExpr {
    Expr::cust_with_values(
        "EXISTS (SELECT 1 FROM unnest($1::text[]) prefix
         WHERE starts_with(scope_requests.name, prefix))",
        [prefixes.iter().cloned().collect::<Vec<_>>()],
    )
}

#[cfg(test)]
mod tests {
    use super::super::tests::{postgres_store, start_public_request};
    use crate::db::AddRequestInviteeCommand;
    use scope_domain::requests::GitFetchCandidates;

    #[tokio::test]
    async fn fetch_candidates_select_only_matching_open_requests() {
        let store = postgres_store(1).await;
        for id in ["alpha", "alpha_two", "beta", "gamma", "closed"] {
            start_public_request(&store, id).await;
            store
                .admin()
                .execute_for_tests(&format!(
                    "UPDATE scope_requests SET submitted_at_unix = 3, updated_at_unix = 4 WHERE id = '{id}'"
                ))
                .await
                .unwrap();
        }
        store
            .admin()
            .execute_for_tests(
                "UPDATE scope_requests SET head_oid = 'head-' || id;
                 UPDATE scope_requests SET author_user_id = 'user_target_0' WHERE id = 'gamma';
                 UPDATE scope_requests SET closed_at_unix = 4 WHERE id = 'closed'",
            )
            .await
            .unwrap();
        store
            .requests()
            .add_request_invitee(AddRequestInviteeCommand {
                request_id: "beta".into(),
                actor_user_id: "user_author".into(),
                target_handle: "target-0".into(),
                now_unix: 3,
            })
            .await
            .unwrap();
        let listing = |own: Option<&[&str]>, names: &[&str]| GitFetchCandidates::Listing {
            own_name_prefixes: own.map(|own| own.iter().map(|name| name.to_string()).collect()),
            names: names.iter().map(|name| name.to_string()).collect(),
        };
        let requests = store.requests();
        for (viewer, candidates, advertised, expected) in [
            (
                Some("user_author"),
                listing(None, &[]),
                vec![],
                vec![("alpha", false), ("alpha_two", false), ("beta", false)],
            ),
            (
                Some("user_author"),
                listing(Some(&["alpha"]), &[]),
                vec![],
                vec![("alpha", false), ("alpha_two", false)],
            ),
            (
                Some("user_author"),
                listing(Some(&["alpha-t"]), &["gamma", "closed"]),
                vec![],
                vec![("alpha_two", false), ("gamma", false)],
            ),
            (
                Some("user_target_0"),
                listing(None, &[]),
                vec![],
                vec![("beta", true), ("gamma", false)],
            ),
            (None, listing(None, &[]), vec![], vec![]),
            (
                Some("user_author"),
                GitFetchCandidates::Heads(vec!["head-beta".into(), "head-closed".into()]),
                vec!["gamma".to_string()],
                vec![("beta", false), ("gamma", false)],
            ),
            (
                None,
                GitFetchCandidates::Every,
                vec![],
                vec![
                    ("alpha", false),
                    ("alpha_two", false),
                    ("beta", false),
                    ("gamma", false),
                ],
            ),
        ] {
            let selected = requests
                .git_fetch_requests("owner/repo", viewer, &candidates, &advertised)
                .await
                .unwrap();
            assert_eq!(
                selected
                    .iter()
                    .map(|(request, invited)| (request.id.as_str(), *invited))
                    .collect::<Vec<_>>(),
                expected,
                "{viewer:?} {candidates:?}"
            );
        }
    }
}
