use crate::{
    error::DomainError,
    github_account::{GitHubAccount, GitHubAccountType},
    github_connection::{GitHubConnection, ensure_can_manage_github_connection},
    repository::access::RepositoryAccess,
};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

pub const GITHUB_REVIEW_BOTS_LIMIT: usize = 20;
const GITHUB_LOGIN_MAX_CHARS: usize = 39;
const BOT_SUFFIX: &str = "[bot]";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubBotLogin(String);

impl GitHubBotLogin {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        let login = value.trim();
        let Some(name) = login.strip_suffix(BOT_SUFFIX) else {
            return Err(DomainError::invalid_input(format!(
                "\"{login}\" is not a bot. Review bots are GitHub bot logins ending in [bot], like greptile-apps[bot]; people who can write to the repository are imported without being listed."
            )));
        };
        let valid = !name.is_empty()
            && name.chars().count() <= GITHUB_LOGIN_MAX_CHARS
            && !name.starts_with('-')
            && !name.ends_with('-')
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-');
        if valid {
            Ok(Self(login.to_string()))
        } else {
            Err(DomainError::invalid_input(format!(
                "\"{login}\" is not a GitHub bot login. Use a name like greptile-apps[bot]."
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn names(&self, login: &str) -> bool {
        self.0.eq_ignore_ascii_case(login)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubReviewBot {
    pub login: GitHubBotLogin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubReviewBotInput {
    pub login: String,
}

pub fn set_github_review_bots(
    access: RepositoryAccess,
    connection: Option<&GitHubConnection>,
    inputs: Vec<GitHubReviewBotInput>,
) -> Result<Vec<GitHubReviewBot>, DomainError> {
    ensure_can_manage_github_connection(access)?;
    if connection.is_none() {
        return Err(DomainError::not_found(
            "this repository is not connected to GitHub",
        ));
    }
    if inputs.len() > GITHUB_REVIEW_BOTS_LIMIT {
        return Err(DomainError::invalid_input(format!(
            "A repository can list at most {GITHUB_REVIEW_BOTS_LIMIT} review bots."
        )));
    }
    let mut bots = Vec::<GitHubReviewBot>::with_capacity(inputs.len());
    for input in inputs {
        let login = GitHubBotLogin::parse(&input.login)?;
        if bots.iter().any(|bot| bot.login.names(login.as_str())) {
            return Err(DomainError::invalid_input(format!(
                "{} is listed twice.",
                login.as_str()
            )));
        }
        bots.push(GitHubReviewBot { login });
    }
    Ok(bots)
}

pub fn github_review_bots_survive(
    previous: Option<&GitHubConnection>,
    next: &GitHubConnection,
) -> bool {
    previous.is_some_and(|previous| {
        previous.is_connected()
            && next.is_connected()
            && previous.installation_id == next.installation_id
            && previous.github_repository_id == next.github_repository_id
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubRepositoryPermission {
    Admin,
    Write,
    Read,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitHubPermissionGrant {
    pub account_id: u64,
    pub permission: GitHubRepositoryPermission,
}

impl GitHubPermissionGrant {
    pub fn belongs_to(&self, author: &GitHubAccount) -> bool {
        self.account_id == author.id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubAuthorStanding {
    ListedReviewBot,
    NeedsPermission,
    Unlisted,
}

pub fn github_author_standing(
    review_bots: &[GitHubReviewBot],
    author: &GitHubAccount,
) -> GitHubAuthorStanding {
    match author.account_type {
        GitHubAccountType::Bot if review_bots.iter().any(|bot| bot.login.names(&author.login)) => {
            GitHubAuthorStanding::ListedReviewBot
        }
        GitHubAccountType::User => GitHubAuthorStanding::NeedsPermission,
        GitHubAccountType::Bot | GitHubAccountType::Organization => GitHubAuthorStanding::Unlisted,
    }
}

pub fn may_import_github_author(
    standing: &GitHubAuthorStanding,
    permission: Option<GitHubRepositoryPermission>,
) -> bool {
    match standing {
        GitHubAuthorStanding::ListedReviewBot => true,
        GitHubAuthorStanding::NeedsPermission => matches!(
            permission,
            Some(GitHubRepositoryPermission::Admin | GitHubRepositoryPermission::Write)
        ),
        GitHubAuthorStanding::Unlisted => false,
    }
}
