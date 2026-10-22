use crate::error::ApiError;
use scope_api_contract::{
    GitHubAccountResponse, RequestDiscussionAuthorResponse, RequestDiscussionResolverResponse,
};
use scope_domain::requests::{RequestDiscussionAuthor, RequestDiscussionResolver};
use scope_postgres::db::RequestDiscussionActors;

pub(super) fn author_response(
    author: &RequestDiscussionAuthor,
    actors: &RequestDiscussionActors,
) -> Result<Option<RequestDiscussionAuthorResponse>, ApiError> {
    Ok(match author {
        RequestDiscussionAuthor::User(user_id) => {
            let user = user(user_id, actors)?;
            Some(RequestDiscussionAuthorResponse::User {
                id: user.id.clone(),
                handle: user.handle.clone(),
            })
        }
        RequestDiscussionAuthor::DeletedUser => None,
        RequestDiscussionAuthor::GitHub {
            github_account_id,
            source_url,
        } => Some(RequestDiscussionAuthorResponse::GitHub {
            account: github_account(*github_account_id, actors)?,
            source_url: source_url.clone(),
        }),
    })
}

pub(super) fn resolver_response(
    resolver: Option<&RequestDiscussionResolver>,
    actors: &RequestDiscussionActors,
) -> Result<Option<RequestDiscussionResolverResponse>, ApiError> {
    resolver
        .map(|resolver| {
            Ok(match resolver {
                RequestDiscussionResolver::User(user_id) => {
                    let user = user(user_id, actors)?;
                    RequestDiscussionResolverResponse::User {
                        id: user.id.clone(),
                        handle: user.handle.clone(),
                    }
                }
                RequestDiscussionResolver::GitHub(github_account_id) => {
                    RequestDiscussionResolverResponse::GitHub {
                        account: github_account(*github_account_id, actors)?,
                    }
                }
            })
        })
        .transpose()
}

fn user<'a>(
    user_id: &str,
    actors: &'a RequestDiscussionActors,
) -> Result<&'a scope_domain::account::UserAccount, ApiError> {
    actors
        .users
        .get(user_id)
        .ok_or_else(|| ApiError::internal_message("request discussion actor was not persisted"))
}

fn github_account(
    github_account_id: u64,
    actors: &RequestDiscussionActors,
) -> Result<GitHubAccountResponse, ApiError> {
    let account = actors
        .github_accounts
        .get(&github_account_id)
        .ok_or_else(|| {
            ApiError::internal_message("request discussion GitHub account is missing")
        })?;
    Ok(GitHubAccountResponse {
        github_user_id: account.id,
        login: account.login.clone(),
        account_type: account.account_type.into(),
        avatar_url: account.avatar_url.clone(),
    })
}
