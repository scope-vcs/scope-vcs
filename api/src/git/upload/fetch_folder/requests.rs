use crate::{error::ApiError, persistence::unix_now, state::AppState};
use scope_domain::{
    repository::access::RepositoryAccess,
    requests::{
        GitFetchAsk, GitFetchHead, Request, RequestViewer, request_readable_in_git,
        retain_git_fetch_heads,
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
    let advertised = match ask {
        GitFetchAsk::Transfer { wants, .. } => {
            state
                .metadata
                .repositories()
                .advertised_git_fetch_heads(reader, wants, unix_now()?)
                .await?
        }
        GitFetchAsk::Advertisement | GitFetchAsk::Listing { .. } => Vec::new(),
    };
    let advertised_request_ids = advertised
        .iter()
        .filter_map(|(_, head)| match head {
            GitFetchHead::Requests(requests) => Some(requests),
            GitFetchHead::Main => None,
        })
        .flatten()
        .map(|request| request.id.clone())
        .collect::<Vec<_>>();
    let mut requests = Vec::new();
    let mut current = Vec::new();
    for (request, invitee) in state
        .metadata
        .requests()
        .git_fetch_requests(
            reader.repo_id,
            reader.user_id,
            &ask.candidates(),
            &advertised_request_ids,
        )
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
    let retained = retain_git_fetch_heads(&current, &mut requests, advertised);
    Ok(FetchRequests { requests, retained })
}

pub(super) async fn split_unprepared(
    state: &AppState,
    reader: &GitFetchReader<'_>,
    ask: &GitFetchAsk,
    full_view: bool,
    selection: &mut FetchRequests,
) -> Result<Vec<Request>, ApiError> {
    let candidates = selection
        .requests
        .iter()
        .filter(|request| ask.lists_without_preparing(request, full_view))
        .map(|request| (request.id.clone(), request.head_oid.clone()))
        .collect::<Vec<_>>();
    let pooled = state
        .metadata
        .repositories()
        .git_pooled_request_heads(reader.repo_id, &candidates)
        .await?;
    let (unprepared, prepared) = std::mem::take(&mut selection.requests)
        .into_iter()
        .partition(|request| {
            ask.lists_without_preparing(request, full_view) && pooled.contains(&request.id)
        });
    selection.requests = prepared;
    Ok(unprepared)
}
