use super::{
    Request, RequestBase, RequestEvent, RequestEventKind, RequestEventPayload, RequestRevision,
    RequestState, advance_request_activity,
};
use crate::{error::DomainError, views::ViewId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const REQUEST_STACK_MAX_ANCESTORS: u32 = 32;
pub const REQUEST_STACK_MAX_DEPTH: u32 = REQUEST_STACK_MAX_ANCESTORS + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStackStatus {
    Main,
    WaitingForParent,
    ParentExtended,
    ParentRewritten,
    ParentClosed,
    AncestorBlocked,
    IntegrationConflict,
}

impl RequestStackStatus {
    pub const fn requires_restack(self) -> bool {
        matches!(
            self,
            Self::ParentRewritten
                | Self::ParentClosed
                | Self::AncestorBlocked
                | Self::IntegrationConflict
        )
    }

    pub const fn blocking_reason(self) -> Option<&'static str> {
        match self {
            Self::Main => None,
            Self::WaitingForParent | Self::ParentExtended => Some("parent request must land first"),
            Self::ParentRewritten => Some("parent no longer contains the pinned revision"),
            Self::ParentClosed => Some("parent request is closed"),
            Self::AncestorBlocked => Some("an ancestor request is blocked"),
            Self::IntegrationConflict => Some("request integration conflicts with main"),
        }
    }
}

pub fn validate_request_stack_base(
    child_id: &str,
    repo_id: &str,
    view: &ViewId,
    base: &RequestBase,
    ancestors: &[Request],
    parent_current_revision: Option<&RequestRevision>,
) -> Result<u32, DomainError> {
    let RequestBase::Request {
        request_id,
        revision_id,
        head_oid,
    } = base
    else {
        if !ancestors.is_empty() {
            return Err(DomainError::conflict("main base cannot have ancestors"));
        }
        return Ok(1);
    };
    if !view.is_private() {
        return Err(DomainError::forbidden(
            "stacks require the private full view",
        ));
    }
    if ancestors.is_empty() || ancestors.len() > REQUEST_STACK_MAX_ANCESTORS as usize {
        return Err(DomainError::conflict(
            "request stack requires a complete ancestry of at most 32 parents",
        ));
    }
    let parent = &ancestors[0];
    let revision = parent_current_revision
        .ok_or_else(|| DomainError::conflict("parent requires a saved current revision"))?;
    if parent.state() != RequestState::Open
        || parent.git_snapshot.is_none()
        || parent.id != *request_id
        || revision.id != *revision_id
        || revision.request_id != parent.id
        || revision.new_head_oid != *head_oid
        || parent.head_oid != *head_oid
    {
        return Err(DomainError::conflict(
            "parent pin must name an open parent's current saved revision",
        ));
    }
    let mut seen = BTreeSet::from([child_id]);
    for (index, ancestor) in ancestors.iter().enumerate() {
        if ancestor.repo_id != repo_id
            || &ancestor.view != view
            || !seen.insert(ancestor.id.as_str())
        {
            return Err(DomainError::conflict(
                "request stack must be acyclic within the same repository and view",
            ));
        }
        match (&ancestor.base, ancestors.get(index + 1)) {
            (RequestBase::Request { request_id, .. }, Some(next)) if request_id == &next.id => {}
            (RequestBase::Main { .. }, None) => {}
            _ => {
                return Err(DomainError::conflict(
                    "request stack ancestry is incomplete",
                ));
            }
        }
    }
    Ok(ancestors.len() as u32 + 1)
}

pub fn validate_request_repin(old: &RequestBase, new: &RequestBase) -> Result<(), DomainError> {
    match (old, new) {
        (
            RequestBase::Request {
                request_id: old_id, ..
            },
            RequestBase::Request {
                request_id: new_id, ..
            },
        ) if old_id == new_id => Ok(()),
        _ if old == new => Ok(()),
        _ => Err(DomainError::conflict(
            "request publication may only repin its existing parent",
        )),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestParentGitFacts {
    pub parent_contains_pin: bool,
    pub ancestor_blocked: bool,
    pub integration_conflict: bool,
}

pub fn classify_request_stack(
    base: &RequestBase,
    parent: Option<&Request>,
    facts: RequestParentGitFacts,
) -> Result<RequestStackStatus, DomainError> {
    let RequestBase::Request {
        request_id,
        head_oid,
        ..
    } = base
    else {
        return Ok(RequestStackStatus::Main);
    };
    let parent = parent
        .filter(|parent| parent.id == *request_id)
        .ok_or_else(|| DomainError::conflict("request parent is unavailable"))?;
    if parent.state() == RequestState::Closed {
        return Ok(RequestStackStatus::ParentClosed);
    }
    if !facts.parent_contains_pin {
        return Ok(RequestStackStatus::ParentRewritten);
    }
    if facts.ancestor_blocked {
        return Ok(RequestStackStatus::AncestorBlocked);
    }
    if facts.integration_conflict {
        return Ok(RequestStackStatus::IntegrationConflict);
    }
    Ok(if parent.head_oid == *head_oid {
        RequestStackStatus::WaitingForParent
    } else {
        RequestStackStatus::ParentExtended
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestStackRefreshEffects {
    pub stop_auto_merge: bool,
    pub refresh_queue: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestStackRefreshMutation {
    pub request: Request,
    pub effects: RequestStackRefreshEffects,
}

pub fn mark_request_integration_conflict(
    mut request: Request,
    expected_head_oid: &str,
    expected_base_version: u64,
    now_unix: u64,
) -> Result<RequestStackRefreshMutation, DomainError> {
    if request.is_terminal()
        || request.head_oid != expected_head_oid
        || request.base_version != expected_base_version
    {
        return Err(DomainError::conflict("request integration context changed"));
    }
    if !request.view.is_private() {
        return Ok(RequestStackRefreshMutation {
            request,
            effects: RequestStackRefreshEffects {
                stop_auto_merge: false,
                refresh_queue: false,
            },
        });
    }
    let changed = request.stack_status != RequestStackStatus::IntegrationConflict;
    request.stack_status = RequestStackStatus::IntegrationConflict;
    request.updated_at_unix = request.updated_at_unix.max(now_unix);
    if changed {
        advance_request_activity(&mut request)?;
    }
    request.validate_facts()?;
    Ok(RequestStackRefreshMutation {
        request,
        effects: RequestStackRefreshEffects {
            stop_auto_merge: true,
            refresh_queue: changed,
        },
    })
}

pub fn refresh_request_stack(
    mut request: Request,
    parent: Option<&Request>,
    facts: RequestParentGitFacts,
    now_unix: u64,
) -> Result<RequestStackRefreshMutation, DomainError> {
    if matches!(request.base, RequestBase::Main { .. }) {
        return Ok(RequestStackRefreshMutation {
            request,
            effects: RequestStackRefreshEffects {
                stop_auto_merge: false,
                refresh_queue: false,
            },
        });
    }
    let parent = parent.ok_or_else(|| DomainError::conflict("request parent is unavailable"))?;
    if parent.repo_id != request.repo_id || parent.view != request.view {
        return Err(DomainError::conflict(
            "request parent must share the repository and view",
        ));
    }
    let status = classify_request_stack(&request.base, Some(parent), facts)?;
    let depth = parent
        .stack_depth
        .checked_add(1)
        .filter(|depth| *depth <= REQUEST_STACK_MAX_DEPTH)
        .ok_or_else(|| DomainError::conflict("request stack exceeds 32 ancestors"))?;
    let depth_changed = request.stack_depth != depth;
    let status_changed = request.stack_status != status;
    let refresh_queue = depth_changed
        || (status_changed
            && (request.stack_status.requires_restack() || status.requires_restack()));
    if depth_changed || status_changed {
        request.stack_status = status;
        request.stack_depth = depth;
        request.updated_at_unix = request.updated_at_unix.max(now_unix);
        if refresh_queue {
            advance_request_activity(&mut request)?;
        }
        request.validate_facts()?;
    }
    Ok(RequestStackRefreshMutation {
        request,
        effects: RequestStackRefreshEffects {
            stop_auto_merge: status.requires_restack(),
            refresh_queue,
        },
    })
}

#[derive(Clone, Debug)]
pub struct RetargetRequestAfterParentMergeInput {
    pub parent_head_oid: String,
    pub event_id: String,
    pub now_unix: u64,
    pub parent_contains_pin: bool,
    pub contained_main_oid: Option<String>,
    pub contained_main_descends_from_ancestor: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestRetargetEffects {
    pub rebind_check_context: bool,
    pub preserve_auto_merge_intent: bool,
    pub refresh_queue: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestRetargetMutation {
    pub request: Request,
    pub event: RequestEvent,
    pub effects: RequestRetargetEffects,
}

pub fn retarget_request_after_parent_merge(
    mut child: Request,
    parent: &Request,
    input: RetargetRequestAfterParentMergeInput,
) -> Result<RequestRetargetMutation, DomainError> {
    let RequestBase::Request {
        request_id,
        head_oid,
        ..
    } = &child.base
    else {
        return Err(DomainError::conflict("request has no parent pin"));
    };
    if child.is_terminal()
        || parent.id != *request_id
        || parent.repo_id != child.repo_id
        || parent.view != child.view
        || parent.state() != RequestState::Merged
        || parent.merged_head_oid.as_deref() != Some(input.parent_head_oid.as_str())
        || !input.parent_contains_pin
    {
        return Err(DomainError::conflict(
            "only a containing parent merge can retarget an active child",
        ));
    }
    super::validate_required("retarget event id", &input.event_id)?;
    let landed_head_oid = parent
        .merged_head_oid
        .clone()
        .ok_or_else(|| DomainError::conflict("parent merge head is missing"))?;
    let main_oid = parent
        .merged_main_oid
        .clone()
        .ok_or_else(|| DomainError::conflict("parent merge main is missing"))?;
    let old_base = child.base.clone();
    child.base = RequestBase::Main {
        review_base_oid: head_oid.clone(),
    };
    child.base_version = child
        .base_version
        .checked_add(1)
        .ok_or_else(|| DomainError::conflict("request base version overflow"))?;
    if child.stack_status != RequestStackStatus::IntegrationConflict {
        child.stack_status = RequestStackStatus::Main;
    }
    child.stack_depth = 1;
    if input.contained_main_descends_from_ancestor
        && let Some(oid) = input.contained_main_oid
    {
        child.main_ancestor_oid = oid;
    }
    child.updated_at_unix = input.now_unix;
    let position = advance_request_activity(&mut child)?;
    child.validate_facts()?;
    let event = RequestEvent {
        id: input.event_id,
        request_id: child.id.clone(),
        actor_user_id: parent.merged_by_user_id.clone(),
        kind: RequestEventKind::BaseRetargeted,
        position,
        payload: RequestEventPayload::BaseRetargeted {
            old_base,
            landed_head_oid,
            main_oid,
        },
        created_at_unix: input.now_unix,
    };
    Ok(RequestRetargetMutation {
        request: child,
        event,
        effects: RequestRetargetEffects {
            rebind_check_context: true,
            preserve_auto_merge_intent: true,
            refresh_queue: true,
        },
    })
}

#[cfg(test)]
mod tests;
