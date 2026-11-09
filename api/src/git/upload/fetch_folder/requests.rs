use crate::{error::ApiError, persistence::unix_now, state::AppState};
use scope_domain::{
    repository::access::RepositoryAccess,
    requests::{
        GitFetchAsk, Request, RequestViewer, request_readable_in_git, retain_git_fetch_heads,
    },
    views::Views,
};
use scope_postgres::db::GitFetchReader;

pub(super) struct FetchRequests {
    pub(super) requests: Vec<Request>,
    pub(super) retained: Vec<String>,
}

pub(super) async fn readable_requests(
    state: &AppState,
    reader: &GitFetchReader<'_>,
    access: &RepositoryAccess,
    views: &Views,
    ask: &GitFetchAsk,
) -> Result<FetchRequests, ApiError> {
    let readable = |request: &Request, invitee: bool| {
        request_readable_in_git(
            request,
            RequestViewer::new(access.clone(), reader.user_id, invitee),
            views,
            reader.view,
        )
    };
    let mut requests = Vec::new();
    let mut current = Vec::new();
    for (request, invitee) in state
        .metadata
        .requests()
        .git_fetch_requests(reader.repo_id, reader.user_id)
        .await?
    {
        if !readable(&request, invitee) {
            continue;
        }
        if ask.includes(&request, reader.user_id, invitee) {
            requests.push(request.clone());
        }
        current.push(request);
    }
    let advertised = match ask {
        GitFetchAsk::Transfer { wants, .. } => {
            state
                .metadata
                .repositories()
                .advertised_git_fetch_heads(reader, wants, unix_now()?)
                .await?
        }
        GitFetchAsk::Listing { .. } => Vec::new(),
    };
    let retained = retain_git_fetch_heads(&current, &mut requests, advertised);
    Ok(FetchRequests { requests, retained })
}
