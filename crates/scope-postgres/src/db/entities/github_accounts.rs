use super::*;
use scope_domain::github_account::{GitHubAccount, GitHubAccountType};
use scope_domain::requests::{RequestDiscussionAuthor, RequestDiscussionResolver};

pub mod github_account {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_github_accounts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub github_user_id: i64,
        pub login: String,
        pub account_type: String,
        pub avatar_url: Option<String>,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(value: &GitHubAccount, now_unix: u64) -> Result<Self, PostgresError> {
            Ok(Self {
                github_user_id: u64_to_i64(value.id, "GitHub account id")?,
                login: value.login.clone(),
                account_type: encode_enum(value.account_type)?,
                avatar_url: value.avatar_url.clone(),
                updated_at_unix: u64_to_i64(now_unix, "GitHub account update time")?,
            })
        }

        pub fn try_into_domain(self) -> Result<GitHubAccount, PostgresError> {
            GitHubAccount::new(
                i64_to_u64(self.github_user_id, "GitHub account id")?,
                self.login,
                decode_enum::<GitHubAccountType>(self.account_type)?,
                self.avatar_url,
            )
            .map_err(PostgresError::internal)
        }
    }
}

pub struct AuthorColumns {
    pub user_id: Option<String>,
    pub github_user_id: Option<i64>,
    pub source_url: Option<String>,
}

pub fn author_columns(author: &RequestDiscussionAuthor) -> Result<AuthorColumns, PostgresError> {
    Ok(match author {
        RequestDiscussionAuthor::User(user_id) => AuthorColumns {
            user_id: Some(user_id.clone()),
            github_user_id: None,
            source_url: None,
        },
        RequestDiscussionAuthor::DeletedUser => AuthorColumns {
            user_id: None,
            github_user_id: None,
            source_url: None,
        },
        RequestDiscussionAuthor::GitHub {
            github_account_id,
            source_url,
        } => AuthorColumns {
            user_id: None,
            github_user_id: Some(u64_to_i64(*github_account_id, "GitHub account id")?),
            source_url: Some(source_url.clone()),
        },
    })
}

pub fn discussion_author(columns: AuthorColumns) -> Result<RequestDiscussionAuthor, PostgresError> {
    match columns {
        AuthorColumns {
            user_id: Some(user_id),
            github_user_id: None,
            source_url: None,
        } => Ok(RequestDiscussionAuthor::User(user_id)),
        AuthorColumns {
            user_id: None,
            github_user_id: None,
            source_url: None,
        } => Ok(RequestDiscussionAuthor::DeletedUser),
        AuthorColumns {
            user_id: None,
            github_user_id: Some(github_user_id),
            source_url: Some(source_url),
        } => Ok(RequestDiscussionAuthor::GitHub {
            github_account_id: i64_to_u64(github_user_id, "GitHub account id")?,
            source_url,
        }),
        _ => Err(PostgresError::internal_message(
            "request discussion author columns disagree",
        )),
    }
}

pub fn resolver_columns(
    resolver: Option<&RequestDiscussionResolver>,
) -> Result<(Option<String>, Option<i64>), PostgresError> {
    Ok(match resolver {
        None => (None, None),
        Some(RequestDiscussionResolver::User(user_id)) => (Some(user_id.clone()), None),
        Some(RequestDiscussionResolver::GitHub(github_account_id)) => (
            None,
            Some(u64_to_i64(*github_account_id, "GitHub account id")?),
        ),
    })
}

pub fn discussion_resolver(
    user_id: Option<String>,
    github_user_id: Option<i64>,
) -> Result<Option<RequestDiscussionResolver>, PostgresError> {
    match (user_id, github_user_id) {
        (None, None) => Ok(None),
        (Some(user_id), None) => Ok(Some(RequestDiscussionResolver::User(user_id))),
        (None, Some(github_user_id)) => Ok(Some(RequestDiscussionResolver::GitHub(i64_to_u64(
            github_user_id,
            "GitHub account id",
        )?))),
        (Some(_), Some(_)) => Err(PostgresError::internal_message(
            "request discussion resolver columns disagree",
        )),
    }
}
