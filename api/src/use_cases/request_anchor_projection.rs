use crate::{
    error::ApiError,
    git::request_refs::with_request_revision_store_repo,
    state::AppState,
    use_cases::{
        request_diff_file::{
            RequestBlobContent, RequestDiffRange, request_blob_content, request_diff_file,
            request_diff_side_oid,
        },
        request_revision_inspection::{commit_belongs_to_revision, request_commit_visible_paths},
    },
};
use scope_domain::{
    policy::ScopePath,
    repository::{
        RepositoryIncarnation,
        access::{RepositoryAccess, RepositoryAccessContext},
    },
    requests::{
        Request, RequestDiffSide, RequestDiscussion, RequestDiscussionAnchor, RequestLinePosition,
        RequestLineRange, RequestRevision, carry_forward_line_range,
    },
    views::Views,
};
use scope_postgres::db::RepositoryReadPolicy;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AnchorProjection {
    pub(crate) context_visible: bool,
    pub(crate) latest: Option<LatestAnchorPosition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LatestAnchorPosition {
    pub(crate) revision_id: String,
    pub(crate) position: RequestLinePosition,
}

impl AnchorProjection {
    const HIDDEN: Self = Self {
        context_visible: false,
        latest: None,
    };
}

pub(crate) type AnchorProjections = BTreeMap<String, AnchorProjection>;

struct PathAnchor<'a> {
    anchor: &'a RequestDiscussionAnchor,
    path: &'a ScopePath,
    carry_to: Option<&'a RequestRevision>,
}

pub(crate) async fn project_anchors<'a>(
    state: &AppState,
    repo: &RepositoryAccessContext,
    request: &Request,
    discussions: impl IntoIterator<Item = &'a RequestDiscussion>,
) -> AnchorProjections {
    let anchored = discussions
        .into_iter()
        .filter_map(|discussion| Some((discussion.id.as_str(), discussion.anchor.as_ref()?)))
        .collect::<Vec<_>>();
    if anchored.is_empty() {
        return AnchorProjections::new();
    }
    let visible_commits = visible_commits(
        state,
        repo,
        request,
        anchored.iter().map(|(_, anchor)| *anchor),
    )
    .await;
    let latest = if anchored.iter().any(|(_, anchor)| anchor.lines.is_some()) {
        latest_revision(state, request).await
    } else {
        None
    };
    let full_view = repo.access.view == *repo.views.full();
    let mut reader = None;
    let mut projections = AnchorProjections::new();
    for (discussion_id, anchor) in anchored {
        let commit_visible = anchor.commit_oid.as_ref().is_none_or(|commit_oid| {
            visible_commits.contains(&(anchor.revision_id.clone(), commit_oid.clone()))
        });
        let carry_to = latest
            .as_ref()
            .filter(|latest| anchor.lines.is_some() && latest.id != anchor.revision_id);
        let inspected_path = anchor
            .path
            .as_ref()
            .filter(|_| commit_visible && (!full_view || carry_to.is_some()));
        let projection = match inspected_path {
            Some(path) => {
                if reader.is_none() {
                    reader = repository_reader(state, repo, request).await;
                }
                match reader.as_ref() {
                    Some(reader) => {
                        let target = PathAnchor {
                            anchor,
                            path,
                            carry_to,
                        };
                        project_path_anchor(state, reader, request, target).await
                    }
                    None => AnchorProjection::HIDDEN,
                }
            }
            None => AnchorProjection {
                context_visible: commit_visible,
                latest: None,
            },
        };
        projections.insert(discussion_id.to_string(), projection);
    }
    projections
}

async fn latest_revision(state: &AppState, request: &Request) -> Option<RequestRevision> {
    match state
        .metadata
        .requests()
        .latest_request_revision(&request.id)
        .await
    {
        Ok(revision) => revision,
        Err(error) => {
            tracing::warn!(
                request_id = %request.id,
                error = ?error,
                "omitting latest discussion anchor positions because the latest revision is unavailable"
            );
            None
        }
    }
}

async fn repository_reader(
    state: &AppState,
    repo: &RepositoryAccessContext,
    request: &Request,
) -> Option<RepositoryReadPolicy> {
    match state
        .metadata
        .repositories()
        .repository_policy_config(repo)
        .await
    {
        Ok(policy) => Some(policy),
        Err(error) => {
            tracing::warn!(
                request_id = %request.id,
                error = ?error,
                "redacting discussion path anchors because the repository policy is unavailable"
            );
            None
        }
    }
}

async fn project_path_anchor(
    state: &AppState,
    reader: &RepositoryReadPolicy,
    request: &Request,
    target: PathAnchor<'_>,
) -> AnchorProjection {
    let PathAnchor {
        anchor,
        path,
        carry_to,
    } = target;
    let carried = carry_to.zip(anchor.lines);
    let original = match anchor_side(
        state,
        reader,
        request,
        anchor,
        path,
        carried.map(|(_, lines)| lines.side),
    )
    .await
    {
        Ok(Some(original)) => original,
        Ok(None) => return AnchorProjection::HIDDEN,
        Err(error) => {
            tracing::warn!(
                request_id = %request.id,
                revision_id = anchor.revision_id,
                error = ?error,
                "redacting a discussion path anchor because request revision inspection failed"
            );
            return AnchorProjection::HIDDEN;
        }
    };
    let Some((latest, lines)) = carried else {
        return AnchorProjection {
            context_visible: true,
            latest: None,
        };
    };
    let position = read_side(
        state,
        reader,
        request,
        latest,
        SideSelection {
            range: RequestDiffRange::Request,
            path: path.clone(),
            side: Some(lines.side),
        },
        move |newer| carry_forward(lines.range, original, newer),
    )
    .await;
    let latest_position = match position {
        Ok(position) => Some(LatestAnchorPosition {
            revision_id: latest.id.clone(),
            position,
        }),
        Err(error) => {
            tracing::warn!(
                request_id = %request.id,
                revision_id = latest.id,
                error = ?error,
                "omitting a discussion anchor latest position because request revision inspection failed"
            );
            None
        }
    };
    AnchorProjection {
        context_visible: true,
        latest: latest_position,
    }
}

struct SideContent {
    text: Option<String>,
}

fn carry_forward(
    range: RequestLineRange,
    original: SideContent,
    newer: Option<SideContent>,
) -> RequestLinePosition {
    match (original.text, newer) {
        (Some(original), Some(newer)) => {
            carry_forward_line_range(range, &original, newer.text.as_deref())
        }
        _ => RequestLinePosition::Outdated,
    }
}

async fn anchor_side(
    state: &AppState,
    reader: &RepositoryReadPolicy,
    request: &Request,
    anchor: &RequestDiscussionAnchor,
    path: &ScopePath,
    side: Option<RequestDiffSide>,
) -> Result<Option<SideContent>, ApiError> {
    let revision = state
        .metadata
        .requests()
        .request_revision(&request.id, &anchor.revision_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request revision not found"))?;
    let selection = SideSelection {
        range: RequestDiffRange::of_anchor(anchor),
        path: path.clone(),
        side,
    };
    read_side(state, reader, request, &revision, selection, |side| side).await
}

struct SideSelection {
    range: RequestDiffRange,
    path: ScopePath,
    side: Option<RequestDiffSide>,
}

async fn read_side<T: Send + 'static>(
    state: &AppState,
    reader: &RepositoryReadPolicy,
    request: &Request,
    revision: &RequestRevision,
    selection: SideSelection,
    finish: impl FnOnce(Option<SideContent>) -> T + Send + 'static,
) -> Result<T, ApiError> {
    let inspection_state = state.clone();
    let reader = reader.clone();
    with_request_revision_store_repo(
        state,
        &reader.context.incarnation(),
        request,
        revision,
        move |raw_repo, revision| {
            let access = reader.context.access.clone();
            let Some(file) = request_diff_file(
                &inspection_state,
                &reader,
                &access,
                raw_repo,
                revision,
                &selection.range,
                &selection.path,
            )?
            else {
                return Ok(finish(None));
            };
            let text = match selection
                .side
                .and_then(|side| request_diff_side_oid(&file, side))
            {
                Some(oid) => match request_blob_content(raw_repo, oid)? {
                    RequestBlobContent::Text(text) => Some(text),
                    RequestBlobContent::Binary { .. } => None,
                },
                None => None,
            };
            Ok(finish(Some(SideContent { text })))
        },
    )
    .await
}

async fn visible_commits<'a>(
    state: &AppState,
    repo: &RepositoryAccessContext,
    request: &Request,
    anchors: impl IntoIterator<Item = &'a RequestDiscussionAnchor>,
) -> BTreeSet<(String, String)> {
    let commits_by_revision = anchored_commits_by_revision(anchors);
    if commits_by_revision.is_empty() {
        return BTreeSet::new();
    }
    if repo.access.view == *repo.views.full() {
        return flatten(commits_by_revision);
    }
    let policy = match state
        .metadata
        .repositories()
        .repository_policy_config(repo)
        .await
    {
        Ok(policy) => policy,
        Err(error) => {
            tracing::warn!(
                request_id = %request.id,
                error = ?error,
                "redacting discussion anchors because the repository policy is unavailable"
            );
            return BTreeSet::new();
        }
    };
    let incarnation = repo.incarnation();
    let mut visible = BTreeSet::new();
    for (revision_id, commit_oids) in commits_by_revision {
        let result = visible_commits_in_revision(
            state,
            &incarnation,
            RevisionReader {
                policy: policy.clone(),
                views: repo.views.clone(),
                access: repo.access.clone(),
            },
            request,
            &revision_id,
            commit_oids,
        )
        .await;
        match result {
            Ok(commit_oids) => visible.extend(
                commit_oids
                    .into_iter()
                    .map(|commit_oid| (revision_id.clone(), commit_oid)),
            ),
            Err(error) => tracing::warn!(
                request_id = %request.id,
                revision_id,
                error = ?error,
                "redacting discussion anchors because request revision inspection failed"
            ),
        }
    }
    visible
}

fn anchored_commits_by_revision<'a>(
    anchors: impl IntoIterator<Item = &'a RequestDiscussionAnchor>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut commits_by_revision = BTreeMap::<String, BTreeSet<String>>::new();
    for anchor in anchors {
        if let Some(commit_oid) = &anchor.commit_oid {
            commits_by_revision
                .entry(anchor.revision_id.clone())
                .or_default()
                .insert(commit_oid.clone());
        }
    }
    commits_by_revision
}

fn flatten(commits_by_revision: BTreeMap<String, BTreeSet<String>>) -> BTreeSet<(String, String)> {
    commits_by_revision
        .into_iter()
        .flat_map(|(revision_id, commit_oids)| {
            commit_oids
                .into_iter()
                .map(move |commit_oid| (revision_id.clone(), commit_oid))
        })
        .collect()
}

struct RevisionReader {
    policy: scope_postgres::db::RepositoryReadPolicy,
    views: Views,
    access: RepositoryAccess,
}

async fn visible_commits_in_revision(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    reader: RevisionReader,
    request: &Request,
    revision_id: &str,
    commit_oids: BTreeSet<String>,
) -> Result<BTreeSet<String>, ApiError> {
    let Some(revision) = state
        .metadata
        .requests()
        .request_revision(&request.id, revision_id)
        .await?
    else {
        return Ok(BTreeSet::new());
    };
    let RevisionReader {
        policy,
        views,
        access,
    } = reader;
    let inspection_state = state.clone();
    with_request_revision_store_repo(
        state,
        incarnation,
        request,
        &revision,
        move |raw_repo, revision| {
            let mut valid_commits = Vec::new();
            for commit_oid in commit_oids {
                if commit_belongs_to_revision(raw_repo, revision, &commit_oid)? {
                    valid_commits.push(commit_oid);
                }
            }
            if valid_commits.is_empty() {
                return Ok(BTreeSet::new());
            }
            let resolved = crate::use_cases::request_inspection_policy::resolve_commits(
                &inspection_state,
                &policy,
                &access.view,
                raw_repo,
                &valid_commits,
            )?;
            let mut visible = BTreeSet::new();
            for commit_oid in valid_commits {
                let (_, hidden) = request_commit_visible_paths(
                    raw_repo,
                    &resolved,
                    &views,
                    &access,
                    &commit_oid,
                )?;
                if !hidden {
                    visible.insert(commit_oid);
                }
            }
            Ok(visible)
        },
    )
    .await
}
