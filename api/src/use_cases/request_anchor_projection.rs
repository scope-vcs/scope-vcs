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
        Request, RequestDiffSide, RequestDiscussion, RequestDiscussionAnchor,
        RequestDiscussionLines, RequestLinePosition, RequestRevision, carry_forward_line_range,
    },
    views::Views,
};
use scope_postgres::db::RepositoryReadPolicy;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path as FsPath,
};

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

struct PathAnchor {
    discussion_id: String,
    range: RequestDiffRange,
    path: ScopePath,
    carried: Option<RequestDiscussionLines>,
}

struct CarriedAnchor {
    discussion_id: String,
    path: ScopePath,
    lines: RequestDiscussionLines,
    original: Option<String>,
}

pub(crate) async fn project_anchors<'a>(
    state: &AppState,
    repo: &RepositoryAccessContext,
    request: &Request,
    through_position: u64,
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
        latest_revision_through(state, request, through_position).await
    } else {
        None
    };
    let full_view = repo.access.view == *repo.views.full();
    let (mut projections, inspections) = plan_inspections(
        anchored,
        &visible_commits,
        latest.as_ref().map(|latest| latest.id.as_str()),
        full_view,
    );
    if inspections.is_empty() {
        return projections;
    }
    let Some(reader) = repository_reader(state, repo, request).await else {
        for anchors in inspections.into_values() {
            for anchor in anchors {
                projections.insert(anchor.discussion_id, AnchorProjection::HIDDEN);
            }
        }
        return projections;
    };
    let mut carried = Vec::new();
    for (revision_id, anchors) in inspections {
        let ids = anchors
            .iter()
            .map(|anchor| anchor.discussion_id.clone())
            .collect::<Vec<_>>();
        match inspect_revision_anchors(state, &reader, request, &revision_id, anchors).await {
            Ok(inspected) => {
                for (anchor, original) in inspected {
                    let Some(original) = original else {
                        projections.insert(anchor.discussion_id, AnchorProjection::HIDDEN);
                        continue;
                    };
                    projections.insert(
                        anchor.discussion_id.clone(),
                        AnchorProjection {
                            context_visible: true,
                            latest: None,
                        },
                    );
                    if let Some(lines) = anchor.carried {
                        carried.push(CarriedAnchor {
                            discussion_id: anchor.discussion_id,
                            path: anchor.path,
                            lines,
                            original,
                        });
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    request_id = %request.id,
                    revision_id,
                    error = ?error,
                    "redacting discussion path anchors because request revision inspection failed"
                );
                for id in ids {
                    projections.insert(id, AnchorProjection::HIDDEN);
                }
            }
        }
    }
    if let (Some(latest), false) = (latest, carried.is_empty()) {
        match carry_to_latest(state, &reader, request, &latest, carried).await {
            Ok(positions) => {
                for (discussion_id, position) in positions {
                    if let Some(projection) = projections.get_mut(&discussion_id) {
                        projection.latest = Some(LatestAnchorPosition {
                            revision_id: latest.id.clone(),
                            position,
                        });
                    }
                }
            }
            Err(error) => tracing::warn!(
                request_id = %request.id,
                revision_id = latest.id,
                error = ?error,
                "omitting discussion anchor latest positions because request revision inspection failed"
            ),
        }
    }
    projections
}

type InspectionPlan = (AnchorProjections, BTreeMap<String, Vec<PathAnchor>>);

fn plan_inspections(
    anchored: Vec<(&str, &RequestDiscussionAnchor)>,
    visible_commits: &BTreeSet<(String, String)>,
    latest_revision_id: Option<&str>,
    full_view: bool,
) -> InspectionPlan {
    let mut projections = AnchorProjections::new();
    let mut inspections = BTreeMap::<String, Vec<PathAnchor>>::new();
    for (discussion_id, anchor) in anchored {
        let commit_visible = anchor.commit_oid.as_ref().is_none_or(|commit_oid| {
            visible_commits.contains(&(anchor.revision_id.clone(), commit_oid.clone()))
        });
        let carried = anchor
            .lines
            .filter(|_| latest_revision_id.is_some_and(|latest| latest != anchor.revision_id));
        match anchor
            .path
            .as_ref()
            .filter(|_| commit_visible && (!full_view || carried.is_some()))
        {
            Some(path) => inspections
                .entry(anchor.revision_id.clone())
                .or_default()
                .push(PathAnchor {
                    discussion_id: discussion_id.to_string(),
                    range: RequestDiffRange::of_anchor(anchor),
                    path: path.clone(),
                    carried,
                }),
            None => {
                projections.insert(
                    discussion_id.to_string(),
                    AnchorProjection {
                        context_visible: commit_visible,
                        latest: None,
                    },
                );
            }
        }
    }
    (projections, inspections)
}

pub(crate) async fn latest_revision_through(
    state: &AppState,
    request: &Request,
    through_position: u64,
) -> Option<RequestRevision> {
    match state
        .metadata
        .requests()
        .latest_request_revision_through(&request.id, through_position)
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

type InspectedAnchor = (PathAnchor, Option<Option<String>>);

async fn inspect_revision_anchors(
    state: &AppState,
    reader: &RepositoryReadPolicy,
    request: &Request,
    revision_id: &str,
    anchors: Vec<PathAnchor>,
) -> Result<Vec<InspectedAnchor>, ApiError> {
    let revision = state
        .metadata
        .requests()
        .request_revision(&request.id, revision_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request revision not found"))?;
    let inspection_state = state.clone();
    let reader = reader.clone();
    with_request_revision_store_repo(
        state,
        &reader.context.incarnation(),
        request,
        &revision,
        move |raw_repo, revision| {
            anchors
                .into_iter()
                .map(|anchor| {
                    let side = anchor.carried.map(|lines| lines.side);
                    let original = side_text(
                        &inspection_state,
                        &reader,
                        raw_repo,
                        revision,
                        (&anchor.range, &anchor.path, side),
                    )?;
                    Ok((anchor, original))
                })
                .collect()
        },
    )
    .await
}

async fn carry_to_latest(
    state: &AppState,
    reader: &RepositoryReadPolicy,
    request: &Request,
    latest: &RequestRevision,
    anchors: Vec<CarriedAnchor>,
) -> Result<Vec<(String, RequestLinePosition)>, ApiError> {
    let inspection_state = state.clone();
    let reader = reader.clone();
    with_request_revision_store_repo(
        state,
        &reader.context.incarnation(),
        request,
        latest,
        move |raw_repo, revision| {
            anchors
                .into_iter()
                .map(|anchor| {
                    let newer = side_text(
                        &inspection_state,
                        &reader,
                        raw_repo,
                        revision,
                        (
                            &RequestDiffRange::Request,
                            &anchor.path,
                            Some(anchor.lines.side),
                        ),
                    )?;
                    let position = match (anchor.original, newer) {
                        (Some(original), Some(newer)) => carry_forward_line_range(
                            anchor.lines.range,
                            &original,
                            newer.as_deref(),
                        ),
                        _ => RequestLinePosition::Outdated,
                    };
                    Ok((anchor.discussion_id, position))
                })
                .collect()
        },
    )
    .await
}

fn side_text(
    state: &AppState,
    reader: &RepositoryReadPolicy,
    raw_repo: &FsPath,
    revision: &RequestRevision,
    (range, path, side): (&RequestDiffRange, &ScopePath, Option<RequestDiffSide>),
) -> Result<Option<Option<String>>, ApiError> {
    let Some(file) = request_diff_file(
        state,
        reader,
        &reader.context.access,
        raw_repo,
        revision,
        range,
        path,
    )?
    else {
        return Ok(None);
    };
    let text = match side.and_then(|side| request_diff_side_oid(&file, side)) {
        Some(oid) => match request_blob_content(raw_repo, oid)? {
            RequestBlobContent::Text(text) => Some(text),
            RequestBlobContent::Binary { .. } => None,
        },
        None => None,
    };
    Ok(Some(text))
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

#[cfg(test)]
mod tests;
