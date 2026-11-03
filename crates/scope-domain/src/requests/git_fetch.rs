use super::{Request, RequestViewer, canonical_request_ref, request_policy};
use crate::views::{ViewId, Views};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitFetchAsk {
    Listing { named_refs: BTreeSet<String> },
    Transfer { wants: Vec<String> },
}

impl GitFetchAsk {
    pub fn listing() -> Self {
        Self::Listing {
            named_refs: BTreeSet::new(),
        }
    }

    pub fn includes(&self, request: &Request, user: Option<&str>, invitee: bool) -> bool {
        invitee
            || user.is_some_and(|user| request.is_author(user))
            || match self {
                Self::Listing { named_refs } => {
                    named_refs.contains(&canonical_request_ref(&request.name))
                }
                Self::Transfer { wants } => wants.contains(&request.head_oid),
            }
    }

    pub fn is_listing(&self) -> bool {
        matches!(self, Self::Listing { .. })
    }
}

#[derive(Clone, Debug)]
pub enum GitFetchHead {
    Main,
    Requests(Vec<Request>),
}

pub fn request_readable_in_git(
    request: &Request,
    viewer: RequestViewer<'_>,
    views: &Views,
    view: &ViewId,
) -> bool {
    request_policy(request, viewer, views).exact_visible && views.may_read(view, &request.view)
}

pub fn retain_git_fetch_heads(
    readable: &[Request],
    requests: &mut Vec<Request>,
    advertised: Vec<(String, GitFetchHead)>,
) -> Vec<String> {
    let current = readable
        .iter()
        .filter(|request| request.state().advertised_in_git())
        .map(|request| (request.id.clone(), request.view.clone()))
        .collect::<BTreeSet<_>>();
    let mut retained = Vec::new();
    for (oid, head) in advertised {
        let GitFetchHead::Requests(previous) = head else {
            retained.push(oid);
            continue;
        };
        let mut readable = false;
        for old in previous {
            if !current.contains(&(old.id.clone(), old.view.clone())) {
                continue;
            }
            readable = true;
            if !requests
                .iter()
                .any(|request| request.head_oid == old.head_oid)
            {
                requests.push(old);
            }
        }
        if readable {
            retained.push(oid);
        }
    }
    retained
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        repository::access::RepositoryActor,
        requests::fixtures::{access, open_request, working_request},
    };

    #[test]
    fn retained_heads_require_a_current_active_request_in_the_same_view() {
        let old = open_request();
        for change in ["rewrite", "view", "closed", "revoked", "shared-head"] {
            let mut current = old.clone();
            current.head_oid = "rewritten".into();
            let mut requests = match change {
                "view" => {
                    current.view = ViewId::private();
                    vec![current]
                }
                "closed" => {
                    current.closed_at_unix = Some(30);
                    vec![current]
                }
                "revoked" => Vec::new(),
                _ => vec![current],
            };
            let mut saved = vec![old.clone()];
            if change == "shared-head" {
                let mut closed = old.clone();
                closed.id = "closed-association".into();
                saved.push(closed);
            }
            let readable = requests.clone();
            let retained = retain_git_fetch_heads(
                &readable,
                &mut requests,
                vec![(old.head_oid.clone(), GitFetchHead::Requests(saved))],
            );
            let allowed = matches!(change, "rewrite" | "shared-head");
            assert_eq!(retained.contains(&old.head_oid), allowed, "{change}");
            assert_eq!(
                requests
                    .iter()
                    .any(|request| request.head_oid == old.head_oid),
                allowed,
                "{change}"
            );
        }
    }

    #[test]
    fn fetches_list_only_own_invited_named_or_wanted_requests() {
        let request = open_request();
        let named = GitFetchAsk::Listing {
            named_refs: [canonical_request_ref(&request.name)].into(),
        };
        let wanted = GitFetchAsk::Transfer {
            wants: vec![request.head_oid.clone()],
        };
        let other_ref = GitFetchAsk::Listing {
            named_refs: ["refs/heads/".to_string(), "refs/heads/other".to_string()].into(),
        };
        let other_want = GitFetchAsk::Transfer {
            wants: vec!["other".to_string()],
        };
        for (ask, user, invitee, expected) in [
            (&GitFetchAsk::listing(), Some("author"), false, true),
            (&GitFetchAsk::listing(), Some("reader"), true, true),
            (&GitFetchAsk::listing(), Some("reader"), false, false),
            (&GitFetchAsk::listing(), None, false, false),
            (&other_ref, Some("reader"), false, false),
            (&other_want, Some("reader"), false, false),
            (&named, None, false, true),
            (&wanted, Some("reader"), false, true),
        ] {
            assert_eq!(
                ask.includes(&request, user, invitee),
                expected,
                "{ask:?} {user:?} {invitee}"
            );
        }
    }

    #[test]
    fn a_draft_parent_is_readable_only_with_current_author_or_invitee_access() {
        let request = working_request();
        let views = Views::builtin();
        for (user, invitee, expected) in [
            (None, false, false),
            (Some("reader"), false, false),
            (Some("reader"), true, true),
            (Some("author"), false, true),
        ] {
            let viewer = RequestViewer::new(
                access(RepositoryActor::Public, ViewId::public(), false),
                user,
                invitee,
            );
            assert_eq!(
                request_readable_in_git(&request, viewer, &views, &ViewId::public()),
                expected
            );
        }
    }
}
