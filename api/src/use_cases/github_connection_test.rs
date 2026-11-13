use crate::{error::ApiError, state::AppState};
use scope_domain::{
    github_connection::GitHubConnection,
    github_connection_test::{
        GitHubConnectionTestResult, GitHubRepositoryFacts, test_github_connection,
    },
};

pub(crate) async fn run_github_connection_test(
    state: &AppState,
    connection: &GitHubConnection,
) -> Result<Vec<GitHubConnectionTestResult>, ApiError> {
    let app = state.github_app()?;
    let (installation, repository) = tokio::try_join!(
        app.installation(connection.installation_id),
        app.repository(connection.installation_id, connection.github_repository_id),
    )?;
    let repository = repository.map(|repository| GitHubRepositoryFacts {
        full_name: repository.full_name,
        private: repository.private,
    });
    Ok(test_github_connection(
        connection,
        &installation,
        repository.as_ref(),
    ))
}
