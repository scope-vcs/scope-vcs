use crate::{error::ApiError, github::GitHubPermissionAnswer, state::AppState};
use scope_domain::{
    github_account::GitHubAccount,
    github_connection::GitHubConnection,
    github_review_bots::{
        GitHubAuthorStanding, GitHubReviewBot, github_author_standing, may_import_github_author,
    },
};
use std::collections::BTreeMap;

pub(super) struct ImportAuthors<'a> {
    state: &'a AppState,
    connection: &'a GitHubConnection,
    review_bots: Vec<GitHubReviewBot>,
    admitted: BTreeMap<u64, bool>,
    refusals: Vec<String>,
}

impl<'a> ImportAuthors<'a> {
    pub(super) async fn load(
        state: &'a AppState,
        connection: &'a GitHubConnection,
    ) -> Result<Self, ApiError> {
        Ok(Self {
            state,
            connection,
            review_bots: state
                .metadata
                .repositories()
                .github_review_bots(&connection.repository_id)
                .await?,
            admitted: BTreeMap::new(),
            refusals: Vec::new(),
        })
    }

    pub(super) async fn admits(&mut self, author: &GitHubAccount) -> Result<bool, ApiError> {
        if let Some(admitted) = self.admitted.get(&author.id) {
            return Ok(*admitted);
        }
        let standing = github_author_standing(&self.review_bots, author);
        let permission = match standing {
            GitHubAuthorStanding::NeedsPermission => {
                match self
                    .state
                    .github_app()?
                    .repository_permission(
                        self.connection.installation_id,
                        &self.connection.github_full_name,
                        &author.login,
                    )
                    .await?
                {
                    GitHubPermissionAnswer::Known(permission) => Some(permission),
                    GitHubPermissionAnswer::Refused(reason) => {
                        self.refusals.push(reason);
                        None
                    }
                }
            }
            GitHubAuthorStanding::ListedReviewBot | GitHubAuthorStanding::Unlisted => None,
        };
        let admitted = may_import_github_author(&standing, permission);
        self.admitted.insert(author.id, admitted);
        Ok(admitted)
    }

    pub(super) fn refusals(self) -> Option<String> {
        (!self.refusals.is_empty()).then(|| self.refusals.join(" "))
    }
}
