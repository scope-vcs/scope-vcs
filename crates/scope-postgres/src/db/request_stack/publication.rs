use super::{RequestBasePublicationFacts, ValidatedRequestPublication, ancestry};
use crate::{
    db::{
        entities, request_access::request_policy_for_user,
        request_revision_rows::latest_revision_for_request,
    },
    error::PostgresError,
};
use scope_domain::{
    repository::access::RepositoryAccessContext,
    requests::{RecordRequestRevisionInput, Request, RequestBase, validate_request_stack_base},
    views::ViewId,
};
use sea_orm::{ConnectionTrait, DatabaseTransaction, EntityTrait};

pub(super) async fn validate_start<C: ConnectionTrait>(
    conn: &C,
    input: &mut scope_domain::requests::StartRequestInput,
    repo: &RepositoryAccessContext,
) -> Result<u32, PostgresError> {
    let ancestors = ancestry(conn, &input.base).await?;
    authorize_ancestors(conn, repo, &input.author_user_id, &ancestors).await?;
    let revision = match ancestors.first() {
        Some(parent) => latest_revision_for_request(conn, &parent.id).await?,
        None => None,
    };
    let depth = validate_request_stack_base(
        &input.id,
        &input.repo_id,
        &input.view,
        &input.base,
        &ancestors,
        revision.as_ref(),
    )?;
    if let Some(parent) = ancestors.first() {
        input.main_ancestor_oid = parent.main_ancestor_oid.clone();
    }
    Ok(depth)
}

pub(super) async fn validate_publication(
    tx: &DatabaseTransaction,
    request: &Request,
    input: &RecordRequestRevisionInput,
    expected_base_version: u64,
    facts: Option<&RequestBasePublicationFacts>,
    repo: &RepositoryAccessContext,
) -> Result<ValidatedRequestPublication, PostgresError> {
    let selected_base = input.new_base.as_ref().unwrap_or(&request.base);
    let explicit_repin = input.new_base.is_some();
    let git_facts = &input.git_facts;
    if request.base_version != expected_base_version {
        return Err(PostgresError::conflict(
            "request base changed during publication",
        ));
    }
    let expected_version = match facts {
        Some(RequestBasePublicationFacts::Unchanged {
            expected_repo_change_version,
        })
        | Some(RequestBasePublicationFacts::Main {
            expected_repo_change_version,
            ..
        }) => Some(*expected_repo_change_version),
        Some(RequestBasePublicationFacts::Request(parent)) => {
            Some(parent.expected_repo_change_version)
        }
        None => None,
    };
    if expected_version.is_some_and(|version| version != repo.record.change_version) {
        return Err(PostgresError::conflict(
            "repository changed during request publication",
        ));
    }
    if !request.view.is_private()
        && request.stack_status == scope_domain::requests::RequestStackStatus::NeedsMainUpdate
    {
        let main = accepted_main_oid(tx, repo, &request.view).await?;
        if git_facts.contained_main_oid.as_ref() != Some(&main)
            || !git_facts.contained_main_descends_from_base
        {
            return Err(PostgresError::conflict(
                "request must contain current view main before recovering",
            ));
        }
    }
    let RequestBase::Request { request_id, .. } = selected_base else {
        match (explicit_repin, facts) {
            (
                true,
                Some(RequestBasePublicationFacts::Main {
                    main_head_oid,
                    child_contains_main,
                    ..
                }),
            ) => {
                let current_main = accepted_main_oid(tx, repo, &request.view).await?;
                if current_main != *main_head_oid
                    || selected_base.review_base_oid() != main_head_oid
                    || !child_contains_main
                    || git_facts.contained_main_oid.as_ref() != Some(main_head_oid)
                    || !git_facts.contained_main_descends_from_base
                {
                    return Err(PostgresError::conflict(
                        "main changed or publication does not contain selected main",
                    ));
                }
            }
            (false, None | Some(RequestBasePublicationFacts::Unchanged { .. })) => {}
            _ => {
                return Err(PostgresError::conflict(
                    "main publication has invalid Git evidence",
                ));
            }
        }
        let depth = validate_request_stack_base(
            &request.id,
            &request.repo_id,
            &request.view,
            selected_base,
            &[],
            None,
        )?;
        return Ok(ValidatedRequestPublication {
            parent: None,
            depth,
        });
    };
    let Some(RequestBasePublicationFacts::Request(facts)) = facts else {
        return Err(PostgresError::conflict(
            "parent publication requires Git evidence",
        ));
    };
    let ancestors = ancestry(tx, selected_base).await?;
    authorize_ancestors(tx, repo, &input.actor_user_id, &ancestors).await?;
    let parent = ancestors
        .first()
        .ok_or_else(|| PostgresError::conflict("request parent is unavailable"))?;
    let revision = latest_revision_for_request(tx, &parent.id)
        .await?
        .ok_or_else(|| PostgresError::conflict("parent requires a saved current revision"))?;
    if facts.request_id != *request_id
        || facts.revision_id != revision.id
        || facts.head_oid != parent.head_oid
        || facts.base_version != parent.base_version
        || !facts.parent_contains_pin
        || !facts.child_contains_pin
    {
        return Err(PostgresError::conflict(
            "parent changed or publication does not contain its pin",
        ));
    }
    let current_base = RequestBase::Request {
        request_id: parent.id.clone(),
        revision_id: revision.id.clone(),
        head_oid: parent.head_oid.clone(),
    };
    let depth = validate_request_stack_base(
        &request.id,
        &request.repo_id,
        &request.view,
        if explicit_repin {
            selected_base
        } else {
            &current_base
        },
        &ancestors,
        Some(&revision),
    )?;
    Ok(ValidatedRequestPublication {
        parent: Some(parent.clone()),
        depth,
    })
}

async fn authorize_ancestors<C: ConnectionTrait>(
    conn: &C,
    repo: &RepositoryAccessContext,
    actor_user_id: &str,
    ancestors: &[Request],
) -> Result<(), PostgresError> {
    for parent in ancestors {
        if parent.repo_id != repo.record.id
            || !request_policy_for_user(conn, repo, parent, actor_user_id)
                .await?
                .exact_visible
        {
            return Err(PostgresError::not_found("parent request not found"));
        }
    }
    Ok(())
}

pub(super) async fn accepted_main_oid<C: ConnectionTrait>(
    conn: &C,
    repo: &RepositoryAccessContext,
    view: &ViewId,
) -> Result<String, PostgresError> {
    if view == repo.views.full() {
        return entities::git_head::Entity::find_by_id(&repo.record.id)
            .one(conn)
            .await
            .map_err(PostgresError::internal)?
            .map(|head| head.head_oid)
            .ok_or_else(|| PostgresError::conflict("repository has no accepted main"));
    }
    super::super::projection_read_models::live_projection_read_model(
        conn,
        &repo.record.id,
        repo.record.content_version,
        view,
    )
    .await?
    .and_then(|projection| projection.head_oid)
    .ok_or_else(|| PostgresError::conflict("request view main is not ready"))
}
