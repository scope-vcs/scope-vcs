use crate::github::GitHubRepository;
use scope_api_contract::RequestActorSummaryResponse;
use scope_domain::github_connection::{
    GitHubConnectionStatus, GitHubDisconnectReason, GitHubRepositoryVisibility,
};
use scope_postgres::db::GitHubConnectionRead;
use serde::{Deserialize, Serialize};

/// A repository's GitHub connection as its maintainers see it.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubConnectionResponse {
    /// `false` when this server has no GitHub App, so nothing can connect.
    pub(crate) configured: bool,
    pub(crate) connection: Option<GitHubConnectionDetailsResponse>,
    /// The check names GitHub must pass before a request merges.
    pub(crate) required_checks: Vec<String>,
    /// Whether the viewer may confirm that a public GitHub repository
    /// receives what Scope pushes, private requests included.
    pub(crate) can_confirm_public: bool,
}

/// The whole list of required check names, replacing the stored one.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct SetGitHubRequiredChecksRequest {
    pub(crate) names: Vec<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubConnectionDetailsResponse {
    pub(crate) github_full_name: String,
    pub(crate) github_url: String,
    /// `None` once that account was deleted.
    pub(crate) connected_by: Option<RequestActorSummaryResponse>,
    pub(crate) connected_at_unix: u64,
    /// Set when GitHub took the repository away from Scope.
    pub(crate) disconnected: Option<GitHubDisconnectionResponse>,
    /// Everything Scope pushes to a public GitHub repository is public.
    pub(crate) public_on_github: bool,
    /// False while a repository that became public waits for a maintainer to
    /// confirm; private requests are not sent there meanwhile.
    pub(crate) public_confirmed: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubDisconnectionResponse {
    pub(crate) reason: GitHubDisconnectReasonResponse,
    pub(crate) at_unix: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(rename_all = "snake_case"))]
pub(crate) enum GitHubDisconnectReasonResponse {
    AppUninstalled,
    InstallationSuspended,
    RepositoryRemoved,
}

/// Starts authorizing the app for a repository.
#[derive(Debug, Default, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubAuthorizeRequest {
    /// The origin of the page that started connecting. GitHub returns there
    /// when it is an allowed Scope web origin, so a development stack reached
    /// at another address works. Without it, the public app origin is used.
    #[serde(default)]
    pub(crate) web_origin: Option<String>,
}

/// GitHub's OAuth screen for the app, carrying a signed setup state.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubAuthorizeResponse {
    pub(crate) authorize_url: String,
}

/// What GitHub's OAuth screen sent back to the setup page.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubSetupRequest {
    pub(crate) state: String,
    pub(crate) code: String,
}

/// The GitHub repositories the maintainer can push through the app, and the
/// grant that connecting one of them requires.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubSetupResponse {
    pub(crate) owner_handle: String,
    pub(crate) repo_name: String,
    pub(crate) repositories: Vec<GitHubRepositoryResponse>,
    /// Where to install the app on a repository that is missing.
    pub(crate) install_url: String,
    pub(crate) grant: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubRepositoryResponse {
    pub(crate) id: u64,
    pub(crate) full_name: String,
    pub(crate) private: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct ConnectGitHubRepositoryRequest {
    pub(crate) grant: String,
    pub(crate) github_repository_id: u64,
    /// Required for a public GitHub repository: everything Scope pushes there,
    /// private requests and private files included, becomes public.
    #[serde(default)]
    pub(crate) acknowledge_public: bool,
}

pub(crate) fn github_connection_response(
    configured: bool,
    read: Option<GitHubConnectionRead>,
    required_checks: Vec<String>,
    can_confirm_public: bool,
) -> GitHubConnectionResponse {
    let connection = read.filter(|_| configured).map(|read| {
        let connection = read.connection;
        let (public_on_github, public_confirmed) = match connection.visibility {
            GitHubRepositoryVisibility::Private => (false, true),
            GitHubRepositoryVisibility::Public { acknowledged } => (true, acknowledged),
        };
        GitHubConnectionDetailsResponse {
            public_on_github,
            public_confirmed,
            github_url: format!("https://github.com/{}", connection.github_full_name),
            github_full_name: connection.github_full_name,
            connected_by: connection
                .connected_by
                .zip(read.connected_by_handle)
                .map(|(id, handle)| RequestActorSummaryResponse { id, handle }),
            connected_at_unix: connection.connected_at_unix,
            disconnected: match connection.status {
                GitHubConnectionStatus::Connected => None,
                GitHubConnectionStatus::Disconnected { reason, at_unix } => {
                    Some(GitHubDisconnectionResponse {
                        reason: match reason {
                            GitHubDisconnectReason::AppUninstalled => {
                                GitHubDisconnectReasonResponse::AppUninstalled
                            }
                            GitHubDisconnectReason::InstallationSuspended => {
                                GitHubDisconnectReasonResponse::InstallationSuspended
                            }
                            GitHubDisconnectReason::RepositoryRemoved => {
                                GitHubDisconnectReasonResponse::RepositoryRemoved
                            }
                        },
                        at_unix,
                    })
                }
            },
        }
    });
    GitHubConnectionResponse {
        configured,
        connection,
        required_checks,
        can_confirm_public,
    }
}

pub(crate) fn github_repository_response(repository: GitHubRepository) -> GitHubRepositoryResponse {
    GitHubRepositoryResponse {
        id: repository.id,
        full_name: repository.full_name,
        private: repository.private,
    }
}
