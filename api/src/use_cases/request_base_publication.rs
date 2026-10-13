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
use scope_postgres::db::{
    RequestBasePublicationFacts, RequestParentPublicationFacts, RequestStackChildFacts,
};
use std::path::Path;

mod headers;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestBasePublication {
    pub(crate) base: RequestBase,
    pub(crate) expected_base_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestBasePublicationHeaders {
    pub(crate) base: RequestBase,
    pub(crate) expected_base_version: u64,
    pub(crate) expected_parent_base_version: Option<u64>,
}

pub(crate) async fn prepare_request_base_publication(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    headers: Option<RequestBasePublicationHeaders>,
    staging_repo: &Path,
    new_head_oid: &str,
    expected_repo_change_version: u64,
) -> Result<
    (
        Option<RequestBasePublication>,
        Option<RequestBasePublicationFacts>,
    ),
    ApiError,
> {
    if let Some(headers) = &headers {
        if headers.expected_base_version != request.base_version {
            return Err(ApiError::conflict(
                "request base publication context changed; refresh and retry",
            ));
        }
    }
    let selected_base = headers
        .as_ref()
        .map_or(&request.base, |headers| &headers.base);
    let RequestBase::Request {
        request_id,
        head_oid,
        ..
    } = selected_base
    else {
        let Some(headers) = headers else {
            return Ok((
                None,
                Some(RequestBasePublicationFacts::Unchanged {
                    expected_repo_change_version,
                }),
            ));
        };
        if headers.expected_parent_base_version.is_some() {
            return Err(ApiError::bad_request(
                "main selection cannot carry a parent version",
            ));
        }
        let RequestBase::Main { review_base_oid } = &headers.base else {
            return Err(ApiError::bad_request("main selection requires a main base"));
        };
        let main_head =
            restore_request_main(state, incarnation, &request.view, staging_repo).await?;
        if review_base_oid != &main_head {
            return Err(ApiError::conflict(
                "main moved; refresh and retry base publication",
            ));
        }
        let path = staging_repo.to_path_buf();
        let main = main_head.clone();
        let head = new_head_oid.to_string();
        let contains = blocking::run(move || {
            git_is_ancestor(&path, &main, &head, "checking selected main ancestry")
        })
        .await?;
        if !contains {
            return Err(ApiError::conflict(
                "request branch must contain its selected main",
            ));
        }
        return Ok((
            Some(RequestBasePublication {
                base: headers.base,
                expected_base_version: headers.expected_base_version,
            }),
            Some(RequestBasePublicationFacts::Main {
                main_head_oid: main_head,
                child_contains_main: true,
                expected_repo_change_version,
            }),
        ));
    };
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
    let current_base = RequestBase::Request {
        request_id: parent.id.clone(),
        revision_id: revision.id.clone(),
        head_oid: parent.head_oid.clone(),
    };
    if revision.new_head_oid != parent.head_oid {
        return Err(ApiError::conflict(
            "parent request revision changed; retry publication",
        ));
    }
    if let Some(headers) = &headers
        && (headers.base != current_base
            || headers.expected_parent_base_version != Some(parent.base_version))
    {
        return Err(ApiError::conflict(
            "request base publication context changed; refresh and retry",
        ));
    }
    restore_request_objects(state, incarnation, &parent, &revision, staging_repo).await?;
    if headers.is_none() {
        restore_request_pin(state, incarnation, request, staging_repo).await?;
    }
    let pin = head_oid.clone();
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
    if !parent_contains_pin {
        return Err(ApiError::conflict(
            "parent history was rewritten; publish an explicit base selection",
        ));
    }
    if !child_contains_pin {
        return Err(ApiError::conflict(
            "request branch must contain its selected parent pin",
        ));
    }
    let facts = RequestBasePublicationFacts::Request(RequestParentPublicationFacts {
        request_id: parent.id,
        revision_id: revision.id,
        head_oid: parent.head_oid,
        base_version: parent.base_version,
        parent_contains_pin,
        child_contains_pin,
        expected_repo_change_version,
    });
    Ok((
        headers.map(|headers| RequestBasePublication {
            base: headers.base,
            expected_base_version: headers.expected_base_version,
        }),
        Some(facts),
    ))
}

pub(crate) async fn restore_current_main(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    staging_repo: &Path,
) -> Result<String, ApiError> {
    let (head, spans) = state
        .metadata
        .repositories()
        .repository_content_source(incarnation)
        .await?;
    let head = head.ok_or_else(|| ApiError::conflict("repository has no accepted main"))?;
    let source = state
        .repository_engine
        .materialize_repository(state, incarnation, &head, &spans)
        .await?;
    let main = head.head_oid.clone();
    let path = staging_repo.to_path_buf();
    blocking::run(move || {
        let refspec = format!("{}:refs/scope/internal/base-main", head.head_oid);
        run_git(
            Some(&path),
            &[
                "fetch",
                "--no-tags",
                source.as_ref().to_string_lossy().as_ref(),
                &refspec,
            ],
            "restoring selected main objects",
        )?;
        run_git(
            Some(&path),
            &["update-ref", "-d", "refs/scope/internal/base-main"],
            "removing main base preparation ref",
        )
    })
    .await?;
    Ok(main)
}

pub(crate) async fn restore_request_main(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    view: &scope_domain::views::ViewId,
    staging_repo: &Path,
) -> Result<String, ApiError> {
    if view.is_private() {
        return restore_current_main(state, incarnation, staging_repo).await;
    }
    let git = crate::git::repository_git::RepositoryGit::load(state, incarnation).await?;
    let views = git.views(state).await?;
    let head = git
        .view_head(state, view)
        .await?
        .ok_or_else(|| ApiError::conflict("request view has no accepted main"))?;
    let source = git.view_repo_at(state, &views, view, Some(&head)).await?;
    let path = staging_repo.to_path_buf();
    let main = head.clone();
    blocking::run(move || {
        let refspec = format!("{main}:refs/scope/internal/base-view-main");
        run_git(
            Some(&path),
            &[
                "fetch",
                "--no-tags",
                source.to_string_lossy().as_ref(),
                &refspec,
            ],
            "restoring request view main objects",
        )?;
        run_git(
            Some(&path),
            &["update-ref", "-d", "refs/scope/internal/base-view-main"],
            "removing request view main preparation ref",
        )
    })
    .await?;
    Ok(head)
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
            parent_head_oid: new_head_oid.to_string(),
            request_id: child.id,
            head_oid: child.head_oid,
            base_version: child.base_version,
            parent_contains_pin,
            contained_main_oid: None,
            accepted_view_main_oid: None,
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
