use super::{Request, RequestEvent};
use crate::error::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestRevision {
    pub id: String,
    pub request_id: String,
    pub position: u64,
    pub actor_user_id: Option<String>,
    pub old_head_oid: String,
    pub new_head_oid: String,
    pub base: super::RequestBase,
    pub base_version: u64,
    pub main_ancestor_oid: String,
    pub rewrote_history: bool,
    pub created_at_unix: u64,
}

impl RequestRevision {
    pub fn comparison_oids(&self) -> Vec<String> {
        [
            &self.new_head_oid,
            &self.old_head_oid,
            self.base.review_base_oid(),
            &self.main_ancestor_oid,
        ]
        .into_iter()
        .filter(|oid| *oid != crate::repository::git::EMPTY_GIT_OID)
        .map(str::to_string)
        .collect()
    }

    pub fn restoration_pack_owner(
        &self,
        request: &Request,
        views: &crate::views::Views,
    ) -> Result<crate::repository::git::GitPoolPackOwner, DomainError> {
        if self.request_id != request.id
            || request.state() != super::RequestState::Merged
            || views.get(&request.view).is_none()
            || !self.comparison_oids().contains(&self.new_head_oid)
        {
            return Err(DomainError::conflict(
                "pool pack restoration requires an existing merged request revision and view",
            ));
        }
        Ok(crate::repository::git::GitPoolPackOwner::Request {
            request_id: request.id.clone(),
            scope: crate::repository::git::RequestPackScope::for_view(&request.view, views),
        })
    }
}

pub fn select_request_review_revision<'a>(
    revisions: &'a [RequestRevision],
    pinned_revision_id: Option<&str>,
) -> Result<Option<&'a RequestRevision>, DomainError> {
    if let Some(pinned_revision_id) = pinned_revision_id {
        return revisions
            .iter()
            .find(|revision| revision.id == pinned_revision_id)
            .map(Some)
            .ok_or_else(|| DomainError::not_found("request revision not found"));
    }
    Ok(revisions.iter().max_by(|left, right| {
        left.position
            .cmp(&right.position)
            .then_with(|| left.id.cmp(&right.id))
    }))
}

pub(super) fn revision(
    request: &Request,
    event: &RequestEvent,
    old_head_oid: String,
    new_head_oid: String,
    rewrote_history: bool,
) -> RequestRevision {
    RequestRevision {
        id: event.id.clone(),
        request_id: request.id.clone(),
        position: event.position,
        actor_user_id: event.actor_user_id.clone(),
        old_head_oid,
        new_head_oid,
        base: request.base.clone(),
        base_version: request.base_version,
        main_ancestor_oid: request.main_ancestor_oid.clone(),
        rewrote_history,
        created_at_unix: event.created_at_unix,
    }
}
