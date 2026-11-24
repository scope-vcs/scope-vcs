use super::{Request, RequestActor, RequestViewer, request_policy, validate_required};
use crate::{error::DomainError, repository::access::RepositoryAccess, views::Views};
use std::collections::BTreeMap;

pub const REQUEST_RESOLVES_LIMIT: usize = 20;
pub const NAMED_REQUEST_NOT_FOUND: &str = "the request you named was not found";
pub(super) const RESOLVE_INSTEAD_OF_ATTACHING: &str =
    "start a request of your own with --resolves instead";

pub fn ensure_named_requests_within_limit(named: usize) -> Result<(), DomainError> {
    if named > REQUEST_RESOLVES_LIMIT {
        return Err(DomainError::invalid_input(format!(
            "name at most {REQUEST_RESOLVES_LIMIT} requests at a time"
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestLink {
    pub resolving_request_id: String,
    pub resolved_request_id: String,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedRequest {
    id: String,
    repo_id: String,
    name: String,
}

impl NamedRequest {
    pub fn seen_by_actor(
        request: &Request,
        actor: RequestActor<'_>,
        actor_user_id: &str,
    ) -> Result<Self, DomainError> {
        if !actor.policy(request, actor_user_id).exact_visible {
            return Err(DomainError::not_found(NAMED_REQUEST_NOT_FOUND));
        }
        Ok(Self {
            id: request.id.clone(),
            repo_id: request.repo_id.clone(),
            name: request.name.clone(),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn named_by(&self, request: &Request, itself: &str) -> Result<&str, DomainError> {
        if self.repo_id != request.repo_id {
            return Err(DomainError::not_found(NAMED_REQUEST_NOT_FOUND));
        }
        if self.id == request.id {
            return Err(DomainError::invalid_input(itself));
        }
        Ok(&self.id)
    }
}

#[derive(Clone, Debug)]
pub struct EditRequestLinksInput {
    pub actor_user_id: String,
    pub resolve: Vec<NamedRequest>,
    pub stop_resolving: Vec<NamedRequest>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestLinksMutation {
    pub added: Vec<RequestLink>,
    pub removed: Vec<RequestLink>,
}

pub fn edit_request_links(
    request: &Request,
    actor: RequestActor<'_>,
    resolves: &[RequestLink],
    input: EditRequestLinksInput,
) -> Result<RequestLinksMutation, DomainError> {
    validate_required("actor user id", &input.actor_user_id)?;
    if !actor
        .permissions(request, &input.actor_user_id)
        .can_edit_links
    {
        return Err(DomainError::forbidden(
            "only a request's author or a maintainer can change what it resolves",
        ));
    }
    if input.resolve.is_empty() && input.stop_resolving.is_empty() {
        return Err(DomainError::invalid_input(
            "name a request to resolve or to stop resolving",
        ));
    }
    let mut mutation = RequestLinksMutation::default();
    for named in &input.stop_resolving {
        let resolved_id = named.named_by(request, "a request cannot resolve itself")?;
        let link = resolves
            .iter()
            .find(|link| link.resolved_request_id == resolved_id)
            .filter(|link| !mutation.removed.contains(link))
            .ok_or_else(|| {
                DomainError::conflict(format!("this request does not resolve {}", named.name))
            })?;
        mutation.removed.push(link.clone());
    }
    for named in &input.resolve {
        let resolved_id = named.named_by(request, "a request cannot resolve itself")?;
        if resolves
            .iter()
            .chain(&mutation.added)
            .any(|link| link.resolved_request_id == resolved_id)
        {
            return Err(DomainError::conflict(format!(
                "this request already resolves {}",
                named.name
            )));
        }
        mutation.added.push(RequestLink {
            resolving_request_id: request.id.clone(),
            resolved_request_id: resolved_id.to_string(),
            created_at_unix: input.now_unix,
        });
    }
    if resolves.len() + mutation.added.len() - mutation.removed.len() > REQUEST_RESOLVES_LIMIT {
        return Err(DomainError::conflict(format!(
            "a request can resolve at most {REQUEST_RESOLVES_LIMIT} requests"
        )));
    }
    Ok(mutation)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedRequest {
    pub request: Request,
    pub viewer_is_invitee: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoredRequestLinks {
    pub links: Vec<RequestLink>,
    pub requests: Vec<LinkedRequest>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestLinks {
    pub resolves: Vec<LinkedRequest>,
    pub resolved_by: Vec<LinkedRequest>,
    pub close_named: Option<LinkedRequest>,
}

impl StoredRequestLinks {
    pub fn seen_by(
        mut self,
        request: &Request,
        access: &RepositoryAccess,
        viewer_user_id: Option<&str>,
        views: &Views,
    ) -> RequestLinks {
        let visible = self
            .requests
            .into_iter()
            .filter(|linked| linked.request.repo_id == request.repo_id)
            .filter_map(|linked| {
                let viewer =
                    RequestViewer::new(access.clone(), viewer_user_id, linked.viewer_is_invitee);
                let policy = request_policy(&linked.request, viewer, views);
                policy.exact_visible.then(|| {
                    let id = linked.request.id.clone();
                    let linked = if policy.change_visible {
                        linked
                    } else {
                        LinkedRequest {
                            request: linked.request.with_change_hidden(),
                            ..linked
                        }
                    };
                    (id, linked)
                })
            })
            .collect::<BTreeMap<_, _>>();
        self.links.sort_by(|left, right| {
            (
                left.created_at_unix,
                &left.resolving_request_id,
                &left.resolved_request_id,
            )
                .cmp(&(
                    right.created_at_unix,
                    &right.resolving_request_id,
                    &right.resolved_request_id,
                ))
        });
        let mut links = RequestLinks {
            close_named: request
                .close_named_request_id()
                .and_then(|id| visible.get(id).cloned()),
            ..RequestLinks::default()
        };
        for link in &self.links {
            if link.resolving_request_id == request.id
                && let Some(resolved) = visible.get(&link.resolved_request_id)
            {
                links.resolves.push(resolved.clone());
            }
            if link.resolved_request_id == request.id
                && let Some(resolving) = visible.get(&link.resolving_request_id)
            {
                links.resolved_by.push(resolving.clone());
            }
        }
        links
    }
}

#[cfg(test)]
mod tests;
