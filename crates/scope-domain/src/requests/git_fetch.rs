use super::{Request, RequestViewer, request_policy};
use crate::views::{ViewId, Views};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitFetchRequest {
    pub id: String,
    pub view: ViewId,
}

#[derive(Clone, Debug)]
pub enum GitFetchHead {
    Main,
    Requests(Vec<GitFetchRequest>),
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
    requests: &mut Vec<Request>,
    advertised: Vec<(String, GitFetchHead)>,
) -> Vec<String> {
    let current = requests
        .iter()
        .filter(|request| request.state().advertised_in_git())
        .map(|request| ((request.id.clone(), request.view.clone()), request.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut retained = Vec::new();
    for (oid, head) in advertised {
        let GitFetchHead::Requests(previous) = head else {
            retained.push(oid);
            continue;
        };
        let mut readable = false;
        for old in previous {
            let Some(current) = current.get(&(old.id, old.view)) else {
                continue;
            };
            readable = true;
            if !requests.iter().any(|request| request.head_oid == oid) {
                let mut current = current.clone();
                current.head_oid = oid.clone();
                requests.push(current);
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
            let mut saved = vec![GitFetchRequest {
                id: old.id.clone(),
                view: old.view.clone(),
            }];
            if change == "shared-head" {
                let mut closed = old.clone();
                closed.id = "closed-association".into();
                saved.push(GitFetchRequest {
                    id: closed.id,
                    view: closed.view,
                });
            }
            let retained = retain_git_fetch_heads(
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
