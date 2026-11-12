use crate::{
    auth::viewer::Viewer,
    error::ApiError,
    git::request_refs::with_request_revision_store_repo,
    http::{
        file_diffs::review_content_response_for_blob,
        history::repo_and_view,
        request_review::git_blob_content,
        requests::{repo_and_access, visible_request},
        responses::{RepoFileContentResponse, RepoFileResponse},
    },
    state::AppState,
    use_cases::{
        request_inspection_policy,
        request_tree::{request_tree_file, request_tree_files},
    },
};
use scope_domain::{
    history::HistoryTreeFile,
    policy::ScopePath,
    repository::access::RepositoryAccessContext,
    requests::{Request, RequestRevision},
    views::ViewId,
};
use scope_postgres::db::RepositoryReadPolicy;
use std::collections::BTreeSet;

pub(crate) enum RepoVersion {
    Request(String),
    Update(String),
}

impl RepoVersion {
    pub(crate) fn select(
        request: Option<String>,
        update: Option<String>,
    ) -> Result<Option<Self>, ApiError> {
        match (request, update) {
            (Some(_), Some(_)) => Err(ApiError::bad_request(
                "request and update cannot be combined",
            )),
            (Some(request), None) => Ok(Some(Self::Request(request))),
            (None, Some(update)) => Ok(Some(Self::Update(update))),
            (None, None) => Ok(None),
        }
    }
}

pub(crate) struct RepoVersionRead<'a> {
    pub(crate) state: &'a AppState,
    pub(crate) viewer: &'a Viewer,
    pub(crate) owner: &'a str,
    pub(crate) repo: &'a str,
    pub(crate) view: Option<ViewId>,
}

pub(crate) async fn repo_version_files(
    read: RepoVersionRead<'_>,
    version: RepoVersion,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    match version {
        RepoVersion::Request(request_id) => request_files(read, &request_id).await,
        RepoVersion::Update(entry_id) => {
            let (repo, view) = update_reader(&read).await?;
            let files = update_tree(read.state, &repo, &view, &entry_id).await?;
            Ok(files
                .into_iter()
                .map(|file| RepoFileResponse {
                    path: file.path.as_str().to_string(),
                    oid: file.content.git_oid,
                    tracked: true,
                    label: file.label.into(),
                })
                .collect())
        }
    }
}

pub(crate) async fn repo_version_file(
    read: RepoVersionRead<'_>,
    version: RepoVersion,
    path: ScopePath,
) -> Result<RepoFileContentResponse, ApiError> {
    match version {
        RepoVersion::Request(request_id) => request_file(read, &request_id, path).await,
        RepoVersion::Update(entry_id) => {
            let (repo, view) = update_reader(&read).await?;
            let file = update_tree(read.state, &repo, &view, &entry_id)
                .await?
                .into_iter()
                .find(|file| file.path == path)
                .ok_or_else(|| ApiError::not_found("file not found"))?;
            let head = read
                .state
                .metadata
                .repositories()
                .repository_content_source(&repo.incarnation())
                .await?;
            let content = review_content_response_for_blob(
                read.state,
                &file.content,
                head.as_ref().map(|head| (repo.incarnation(), head)),
            )
            .await?;
            Ok(RepoFileContentResponse {
                path: file.path.as_str().to_string(),
                oid: file.content.git_oid,
                label: file.label.into(),
                size_bytes: file.content.size_bytes,
                content,
            })
        }
    }
}

async fn update_reader(
    read: &RepoVersionRead<'_>,
) -> Result<(RepositoryAccessContext, ViewId), ApiError> {
    repo_and_view(
        read.state,
        read.viewer,
        read.owner,
        read.repo,
        read.view.clone().map(Into::into),
    )
    .await
}

async fn update_tree(
    state: &AppState,
    repo: &RepositoryAccessContext,
    view: &ViewId,
    entry_id: &str,
) -> Result<Vec<HistoryTreeFile>, ApiError> {
    state
        .metadata
        .repositories()
        .repository_history_tree(&repo.incarnation(), view, entry_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("history entry {entry_id} not found")))
}

struct RequestReader {
    repo: RepositoryReadPolicy,
    reader: ViewId,
    request: Request,
    revision: RequestRevision,
}

async fn request_reader(
    read: &RepoVersionRead<'_>,
    request_id: &str,
) -> Result<RequestReader, ApiError> {
    let (repo, viewer_user_id) =
        repo_and_access(read.state, read.viewer, read.owner, read.repo).await?;
    let access = repo.context.access.clone();
    let reader = read.view.clone().unwrap_or_else(|| access.view.clone());
    if !repo.context.can_read_view(&reader) {
        return Err(ApiError::forbidden("view access required"));
    }
    let (request, _) = visible_request(
        read.state,
        &repo.context,
        viewer_user_id.as_deref(),
        request_id,
    )
    .await?;
    let revision = read
        .state
        .metadata
        .requests()
        .latest_request_revision(&request.id)
        .await?
        .ok_or_else(|| ApiError::not_found("request revision not found"))?;
    Ok(RequestReader {
        repo,
        reader,
        request,
        revision,
    })
}

async fn request_files(
    read: RepoVersionRead<'_>,
    request_id: &str,
) -> Result<Vec<RepoFileResponse>, ApiError> {
    let RequestReader {
        repo,
        reader,
        request,
        revision,
    } = request_reader(&read, request_id).await?;
    let state = read.state.clone();
    let incarnation = repo.context.incarnation();
    with_request_revision_store_repo(
        read.state,
        &incarnation,
        &request,
        &revision,
        move |raw_repo, revision| {
            let files = request_tree_files(raw_repo, &revision.new_head_oid)?;
            let paths = files.iter().map(|file| file.path.clone()).collect();
            let policy = request_inspection_policy::resolve_paths(&state, &repo, &reader, paths)?;
            let views = &repo.context.views;
            Ok(files
                .into_iter()
                .filter(|file| policy.can_read(&file.path, &reader, views))
                .map(|file| RepoFileResponse {
                    label: policy.label(&file.path, views).into(),
                    path: file.path.as_str().to_string(),
                    oid: file.oid,
                    tracked: true,
                })
                .collect())
        },
    )
    .await
}

async fn request_file(
    read: RepoVersionRead<'_>,
    request_id: &str,
    path: ScopePath,
) -> Result<RepoFileContentResponse, ApiError> {
    let RequestReader {
        repo,
        reader,
        request,
        revision,
    } = request_reader(&read, request_id).await?;
    let state = read.state.clone();
    let incarnation = repo.context.incarnation();
    with_request_revision_store_repo(
        read.state,
        &incarnation,
        &request,
        &revision,
        move |raw_repo, revision| {
            let policy = request_inspection_policy::resolve_paths(
                &state,
                &repo,
                &reader,
                BTreeSet::from([path.clone()]),
            )?;
            let views = &repo.context.views;
            if !policy.can_read(&path, &reader, views) {
                return Err(ApiError::not_found("file not found"));
            }
            let file = request_tree_file(raw_repo, &revision.new_head_oid, &path)?
                .ok_or_else(|| ApiError::not_found("file not found"))?;
            let content = git_blob_content(raw_repo, &file.oid)?;
            Ok(RepoFileContentResponse {
                label: policy.label(&path, views).into(),
                path: path.as_str().to_string(),
                oid: file.oid,
                size_bytes: file.size_bytes,
                content,
            })
        },
    )
    .await
}
