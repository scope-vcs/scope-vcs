use crate::{
    error::ApiError,
    git::{
        blocking,
        command::{git_is_ancestor, run_git},
        request_refs::with_request_revision_store_repo,
    },
    state::AppState,
};
use axum::http::HeaderMap;
use scope_domain::{
    repository::RepositoryIncarnation,
    requests::{Request, RequestBase, RequestRevision, RequestState, canonical_request_ref},
};
use scope_postgres::db::{RequestParentPublicationFacts, RequestStackChildFacts};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestBasePublication {
    pub(crate) base: RequestBase,
    pub(crate) expected_base_version: u64,
    pub(crate) expected_parent_base_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestBasePublicationHeaders {
    pub(crate) parent_request_id: String,
    pub(crate) parent_revision_id: String,
    pub(crate) expected_base_version: u64,
    pub(crate) expected_parent_base_version: u64,
    pub(crate) expected_parent_head_oid: String,
}

impl RequestBasePublicationHeaders {
    pub(crate) fn parse(headers: &HeaderMap) -> Result<Option<Self>, ApiError> {
        let names = [
            "scope-request-base",
            "scope-request-base-version",
            "scope-request-parent-base-version",
            "scope-request-parent-head",
        ];
        let mut values = Vec::with_capacity(names.len());
        for name in names {
            let mut entries = headers.get_all(name).iter();
            let value = entries.next();
            if entries.next().is_some() {
                return Err(ApiError::bad_request(
                    "duplicate request base publication header",
                ));
            }
            values.push(
                value.map(|value| value.to_str()).transpose().map_err(|_| {
                    ApiError::bad_request("invalid request base publication header")
                })?,
            );
        }
        if values.iter().all(Option::is_none) {
            return Ok(None);
        }
        let values = values
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                ApiError::bad_request("request base publication requires all four headers")
            })?;
        let (parent_request_id, parent_revision_id) = values[0]
            .strip_prefix("request:")
            .and_then(|value| value.split_once('@'))
            .filter(|(request, revision)| valid_id(request) && valid_id(revision))
            .ok_or_else(|| ApiError::bad_request("invalid request base selection"))?;
        let expected_base_version = parse_version(values[1])?;
        let expected_parent_base_version = parse_version(values[2])?;
        let head = values[3];
        if head.len() != 40 || !head.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ApiError::bad_request("invalid request parent head"));
        }
        Ok(Some(Self {
            parent_request_id: parent_request_id.to_string(),
            parent_revision_id: parent_revision_id.to_string(),
            expected_base_version,
            expected_parent_base_version,
            expected_parent_head_oid: head.to_ascii_lowercase(),
        }))
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn parse_version(value: &str) -> Result<u64, ApiError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ApiError::bad_request("invalid request base version"));
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|version| *version > 0)
        .ok_or_else(|| ApiError::bad_request("invalid request base version"))
}

pub(crate) async fn prepare_request_base_publication(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    headers: Option<RequestBasePublicationHeaders>,
    staging_repo: &Path,
    new_head_oid: &str,
) -> Result<
    (
        Option<RequestBasePublication>,
        Option<RequestParentPublicationFacts>,
    ),
    ApiError,
> {
    let RequestBase::Request { request_id, .. } = &request.base else {
        if headers.is_some() {
            return Err(ApiError::conflict(
                "request base publication must select the existing parent",
            ));
        }
        return Ok((None, None));
    };
    if !request.view.is_private() {
        return Err(ApiError::conflict(
            "request stacks require the private view",
        ));
    }
    let parent = state
        .metadata
        .requests()
        .request_by_id(request_id)
        .await?
        .filter(|parent| parent.repo_id == request.repo_id && parent.view == request.view)
        .ok_or_else(|| ApiError::not_found("parent request not found"))?;
    if parent.state() != RequestState::Open {
        return Err(ApiError::conflict(
            "parent request must be open before publishing a child revision",
        ));
    }
    let revision = state
        .metadata
        .requests()
        .latest_request_revision(&parent.id)
        .await?
        .ok_or_else(|| ApiError::conflict("parent request has no current revision"))?;
    if revision.new_head_oid != parent.head_oid {
        return Err(ApiError::conflict(
            "parent request revision changed; retry publication",
        ));
    }
    let publication = if let Some(headers) = headers {
        if headers.parent_request_id != parent.id
            || headers.parent_revision_id != revision.id
            || headers.expected_parent_head_oid != parent.head_oid
            || headers.expected_parent_base_version != parent.base_version
            || headers.expected_base_version != request.base_version
        {
            return Err(ApiError::conflict(
                "request base publication context changed; refresh and retry",
            ));
        }
        Some(RequestBasePublication {
            base: RequestBase::Request {
                request_id: parent.id.clone(),
                revision_id: revision.id.clone(),
                head_oid: parent.head_oid.clone(),
            },
            expected_base_version: headers.expected_base_version,
            expected_parent_base_version: headers.expected_parent_base_version,
        })
    } else {
        None
    };
    restore_request_objects(state, incarnation, &parent, &revision, staging_repo).await?;
    restore_request_pin(state, incarnation, request, staging_repo).await?;
    let pin = publication
        .as_ref()
        .map_or(request.base.review_base_oid(), |publication| {
            publication.base.review_base_oid()
        })
        .to_string();
    let parent_head = parent.head_oid.clone();
    let path = staging_repo.to_path_buf();
    let new_head = new_head_oid.to_string();
    let (parent_contains_pin, child_contains_pin) = blocking::run(move || {
        Ok((
            git_is_ancestor(&path, &pin, &parent_head, "checking parent pin ancestry")?,
            git_is_ancestor(&path, &pin, &new_head, "checking child pin ancestry")?,
        ))
    })
    .await?;
    if !parent_contains_pin && publication.is_none() {
        return Err(ApiError::conflict(
            "parent history was rewritten; publish an explicit base revision",
        ));
    }
    if !child_contains_pin {
        return Err(ApiError::conflict(
            "request branch must contain its selected parent pin",
        ));
    }
    let facts = RequestParentPublicationFacts {
        request_id: parent.id,
        revision_id: revision.id,
        head_oid: parent.head_oid,
        base_version: publication
            .as_ref()
            .map_or(parent.base_version, |publication| {
                publication.expected_parent_base_version
            }),
        parent_contains_pin,
        child_contains_pin,
    };
    Ok((publication, Some(facts)))
}

pub(crate) async fn request_stack_child_publication_facts(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    staging_repo: &Path,
    new_head_oid: &str,
) -> Result<Vec<RequestStackChildFacts>, ApiError> {
    let children = state
        .metadata
        .requests()
        .request_stack_children(&request.id)
        .await?;
    let mut facts = Vec::with_capacity(children.len());
    for child in children {
        restore_request_pin(state, incarnation, &child, staging_repo).await?;
        let pin = child.base.review_base_oid().to_string();
        let head = new_head_oid.to_string();
        let path = staging_repo.to_path_buf();
        let parent_contains_pin = blocking::run(move || {
            git_is_ancestor(&path, &pin, &head, "classifying child parent ancestry")
        })
        .await?;
        facts.push(RequestStackChildFacts {
            request_id: child.id,
            head_oid: child.head_oid,
            base_version: child.base_version,
            parent_contains_pin,
            contained_main_oid: None,
            contained_main_descends_from_ancestor: false,
        });
    }
    Ok(facts)
}

async fn restore_request_objects(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    revision: &RequestRevision,
    staging_repo: &Path,
) -> Result<(), ApiError> {
    let path = staging_repo.to_path_buf();
    let source_ref = canonical_request_ref(&request.name);
    with_request_revision_store_repo(state, incarnation, request, revision, move |source, _| {
        let refspec = format!("+{source_ref}:refs/scope/internal/stack-publication");
        run_git(
            Some(&path),
            &[
                "fetch",
                "--no-tags",
                source.to_string_lossy().as_ref(),
                &refspec,
            ],
            "restoring stack ancestry objects",
        )?;
        run_git(
            Some(&path),
            &["update-ref", "-d", "refs/scope/internal/stack-publication"],
            "removing stack ancestry temporary ref",
        )
    })
    .await
}

async fn restore_request_pin(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    staging_repo: &Path,
) -> Result<(), ApiError> {
    let RequestBase::Request {
        request_id,
        revision_id,
        ..
    } = &request.base
    else {
        return Ok(());
    };
    let parent = state
        .metadata
        .requests()
        .request_by_id(request_id)
        .await?
        .filter(|parent| parent.repo_id == request.repo_id && parent.view == request.view)
        .ok_or_else(|| ApiError::not_found("parent request not found"))?;
    let revision = state
        .metadata
        .requests()
        .request_revision(request_id, revision_id)
        .await?
        .ok_or_else(|| ApiError::conflict("parent pin revision is unavailable"))?;
    restore_request_objects(state, incarnation, &parent, &revision, staging_repo).await
}
