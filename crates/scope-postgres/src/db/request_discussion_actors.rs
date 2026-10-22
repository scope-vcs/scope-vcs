use super::{auth::load_users_by_ids, entities};
use crate::{db::integer_columns::u64_to_i64, error::PostgresError};
use scope_domain::{
    account::UserAccount,
    github_account::GitHubAccount,
    requests::{RequestDiscussionAuthor, RequestDiscussionResolver},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, sea_query::OnConflict,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
pub struct RequestDiscussionActors {
    pub users: BTreeMap<String, UserAccount>,
    pub github_accounts: BTreeMap<u64, GitHubAccount>,
}

#[derive(Default)]
pub(super) struct DiscussionActorIds {
    users: BTreeSet<String>,
    github_accounts: BTreeSet<u64>,
}

impl DiscussionActorIds {
    pub(super) fn author(&mut self, author: &RequestDiscussionAuthor) {
        if let Some(user_id) = author.user_id() {
            self.users.insert(user_id.to_string());
        }
        if let Some(github_account_id) = author.github_account_id() {
            self.github_accounts.insert(github_account_id);
        }
    }

    pub(super) fn resolver(&mut self, resolver: Option<&RequestDiscussionResolver>) {
        if let Some(user_id) = resolver.and_then(RequestDiscussionResolver::user_id) {
            self.users.insert(user_id.to_string());
        }
        if let Some(github_account_id) =
            resolver.and_then(RequestDiscussionResolver::github_account_id)
        {
            self.github_accounts.insert(github_account_id);
        }
    }

    pub(super) async fn load<C: ConnectionTrait>(
        self,
        conn: &C,
    ) -> Result<RequestDiscussionActors, PostgresError> {
        Ok(RequestDiscussionActors {
            users: load_users_by_ids(conn, self.users).await?,
            github_accounts: github_accounts_by_ids(conn, self.github_accounts).await?,
        })
    }
}

pub(super) async fn github_accounts_by_ids<C: ConnectionTrait>(
    conn: &C,
    ids: BTreeSet<u64>,
) -> Result<BTreeMap<u64, GitHubAccount>, PostgresError> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let ids = ids
        .into_iter()
        .map(|id| u64_to_i64(id, "GitHub account id"))
        .collect::<Result<Vec<_>, _>>()?;
    entities::github_account::Entity::find()
        .filter(entities::github_account::Column::GithubUserId.is_in(ids))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| {
            let account = row.try_into_domain()?;
            Ok((account.id, account))
        })
        .collect()
}

pub(super) async fn save_github_accounts<C: ConnectionTrait>(
    conn: &C,
    accounts: &[GitHubAccount],
    now_unix: u64,
) -> Result<(), PostgresError> {
    for account in accounts {
        entities::github_account::Entity::insert(
            entities::github_account::Model::from_domain(account, now_unix)?.into_active_model(),
        )
        .on_conflict(
            OnConflict::column(entities::github_account::Column::GithubUserId)
                .update_columns([
                    entities::github_account::Column::Login,
                    entities::github_account::Column::AccountType,
                    entities::github_account::Column::AvatarUrl,
                    entities::github_account::Column::UpdatedAtUnix,
                ])
                .to_owned(),
        )
        .exec(conn)
        .await?;
    }
    Ok(())
}
