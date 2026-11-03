use crate::{error::ApiError, persistence::unix_now, state::AppState};
use scope_domain::{
    repository::access::RepositoryAccess,
    requests::{Request, RequestViewer, request_readable_in_git, retain_git_fetch_heads},
    views::Views,
};
use scope_postgres::db::GitFetchReader;
use std::collections::BTreeSet;

pub(super) struct FetchRequests {
    pub(super) requests: Vec<Request>,
    pub(super) unadvertised: BTreeSet<String>,
    pub(super) retained: Vec<String>,
}

pub(super) async fn readable_requests(
    state: &AppState,
    reader: &GitFetchReader<'_>,
    access: &RepositoryAccess,
    views: &Views,
    wants: Option<&[String]>,
) -> Result<FetchRequests, ApiError> {
    let readable = |request: &Request, invitee: bool| {
        request_readable_in_git(
            request,
            RequestViewer::new(access.clone(), reader.user_id, invitee),
            views,
            reader.view,
        )
    };
    let mut requests = state
        .metadata
        .requests()
        .git_fetch_requests(reader.repo_id, reader.user_id)
        .await?
        .into_iter()
        .filter_map(|(request, invitee)| readable(&request, invitee).then_some(request))
        .collect::<Vec<_>>();
    let parents = terminal_parents(state, &requests, reader.user_id, &readable).await?;
    let unadvertised = parents
        .iter()
        .map(|request| request.id.clone())
        .collect::<BTreeSet<_>>();
    requests.splice(0..0, parents);
    let advertised = if let Some(wants) = wants {
        state
            .metadata
            .repositories()
            .advertised_git_fetch_heads(reader, wants, unix_now()?)
            .await?
    } else {
        Vec::new()
    };
    let retained = retain_git_fetch_heads(&mut requests, advertised);
    Ok(FetchRequests {
        requests,
        unadvertised,
        retained,
    })
}

async fn terminal_parents(
    state: &AppState,
    requests: &[Request],
    user: Option<&str>,
    readable: &impl Fn(&Request, bool) -> bool,
) -> Result<Vec<Request>, ApiError> {
    let store = state.metadata.requests();
    let mut known = requests
        .iter()
        .map(|request| request.id.clone())
        .collect::<BTreeSet<_>>();
    let mut pending = requests
        .iter()
        .filter(|request| request.git_snapshot.is_none())
        .filter_map(|request| request.base.parent_request_id())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut parents = Vec::new();
    while let Some(id) = pending.pop() {
        if !known.insert(id.clone()) {
            continue;
        }
        let Some(parent) = store.request_by_id(&id).await? else {
            continue;
        };
        let invitee = match user {
            Some(user) => store.request_is_invitee(&id, user).await?,
            None => false,
        };
        if !readable(&parent, invitee) {
            continue;
        }
        if parent.git_snapshot.is_none()
            && let Some(next) = parent.base.parent_request_id()
        {
            pending.push(next.to_string());
        }
        parents.push(parent);
    }
    Ok(parents)
}
