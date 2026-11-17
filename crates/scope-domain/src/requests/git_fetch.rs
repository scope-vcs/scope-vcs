use super::{REQUEST_REF_PREFIX, Request, RequestViewer, canonical_request_ref, request_policy};
use crate::views::{ViewId, Views};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitFetchRequest {
    pub id: String,
    pub view: ViewId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitFetchAsk {
    Advertisement,
    Listing { named_refs: BTreeSet<String> },
    Transfer { wants: Vec<String>, shallow: bool },
}

impl GitFetchAsk {
    pub fn includes(&self, request: &Request, user: Option<&str>, invitee: bool) -> bool {
        match self {
            Self::Advertisement => invitee || user.is_some_and(|user| request.is_author(user)),
            Self::Listing { named_refs } => {
                let name = canonical_request_ref(&request.name);
                if invitee || user.is_some_and(|user| request.is_author(user)) {
                    named_refs.is_empty()
                        || named_refs.iter().any(|prefix| name.starts_with(prefix))
                } else {
                    named_refs.contains(&name)
                }
            }
            Self::Transfer { wants, shallow } => *shallow || wants.contains(&request.head_oid),
        }
    }

    pub fn lists_without_preparing(&self, request: &Request, full_view: bool) -> bool {
        full_view && matches!(self, Self::Listing { .. }) && request.pushed_at_unix.is_some()
    }

    pub fn is_listing(&self) -> bool {
        !matches!(self, Self::Transfer { .. })
    }

    pub fn candidates(&self) -> GitFetchCandidates {
        match self {
            Self::Advertisement => GitFetchCandidates::Listing {
                own_name_prefixes: None,
                names: BTreeSet::new(),
            },
            Self::Listing { named_refs } if named_refs.is_empty() => GitFetchCandidates::Listing {
                own_name_prefixes: None,
                names: BTreeSet::new(),
            },
            Self::Listing { named_refs } => {
                let mut own_name_prefixes = Some(BTreeSet::new());
                let mut names = BTreeSet::new();
                for prefix in named_refs {
                    if let Some(name) = prefix.strip_prefix(REQUEST_REF_PREFIX) {
                        names.insert(name.to_string());
                        if let Some(prefixes) = &mut own_name_prefixes {
                            prefixes.insert(name.to_string());
                        }
                    } else if REQUEST_REF_PREFIX.starts_with(prefix.as_str()) {
                        own_name_prefixes = None;
                    }
                }
                GitFetchCandidates::Listing {
                    own_name_prefixes,
                    names,
                }
            }
            Self::Transfer { shallow: true, .. } => GitFetchCandidates::Every,
            Self::Transfer { wants, .. } => GitFetchCandidates::Heads(wants.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitFetchCandidates {
    Every,
    Listing {
        own_name_prefixes: Option<BTreeSet<String>>,
        names: BTreeSet<String>,
    },
    Heads(Vec<String>),
}

impl GitFetchCandidates {
    pub fn admits(&self, request: &Request, own: bool) -> bool {
        match self {
            Self::Every => true,
            Self::Listing {
                own_name_prefixes,
                names,
            } => {
                names.contains(&request.name)
                    || own
                        && own_name_prefixes.as_ref().is_none_or(|prefixes| {
                            prefixes
                                .iter()
                                .any(|prefix| request.name.starts_with(prefix.as_str()))
                        })
            }
            Self::Heads(heads) => heads.contains(&request.head_oid),
        }
    }
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
    readable: &[Request],
    requests: &mut Vec<Request>,
    advertised: Vec<(String, GitFetchHead)>,
) -> Vec<String> {
    let current = readable
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
                    current.lifecycle.closed_at_unix = Some(30);
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
    fn listings_offer_own_requests_under_asked_prefixes_and_transfers_only_wanted_heads() {
        let request = open_request();
        let named = GitFetchAsk::Listing {
            named_refs: [canonical_request_ref(&request.name)].into(),
        };
        let wanted = GitFetchAsk::Transfer {
            wants: vec![request.head_oid.clone()],
            shallow: false,
        };
        let other_ref = GitFetchAsk::Listing {
            named_refs: ["refs/heads/".to_string(), "refs/heads/other".to_string()].into(),
        };
        let other_want = GitFetchAsk::Transfer {
            wants: vec!["other".to_string()],
            shallow: false,
        };
        let shallow = GitFetchAsk::Transfer {
            wants: vec!["other".to_string()],
            shallow: true,
        };
        let unrelated_prefix = GitFetchAsk::Listing {
            named_refs: ["refs/heads/other".to_string(), "refs/tags/".to_string()].into(),
        };
        let partial_prefix = GitFetchAsk::Listing {
            named_refs: [canonical_request_ref(
                &request.name[..request.name.len() - 1],
            )]
            .into(),
        };
        for (ask, user, invitee, expected) in [
            (&GitFetchAsk::Advertisement, Some("author"), false, true),
            (&GitFetchAsk::Advertisement, Some("reader"), true, true),
            (&GitFetchAsk::Advertisement, Some("reader"), false, false),
            (&GitFetchAsk::Advertisement, None, false, false),
            (&other_ref, Some("author"), false, true),
            (&other_ref, Some("reader"), true, true),
            (&other_ref, Some("reader"), false, false),
            (&unrelated_prefix, Some("author"), false, false),
            (&unrelated_prefix, Some("reader"), true, false),
            (&partial_prefix, Some("author"), false, true),
            (&partial_prefix, Some("reader"), false, false),
            (&other_want, Some("author"), false, false),
            (&other_want, Some("reader"), true, false),
            (&other_want, Some("reader"), false, false),
            (&named, None, false, true),
            (&wanted, Some("reader"), false, true),
            (&wanted, Some("author"), false, true),
            (&shallow, Some("reader"), false, true),
        ] {
            assert_eq!(
                ask.includes(&request, user, invitee),
                expected,
                "{ask:?} {user:?} {invitee}"
            );
        }
    }

    #[test]
    fn only_full_view_v2_listings_list_pushed_requests_without_preparing_them() {
        let pushed = open_request();
        let mut unpushed = pushed.clone();
        unpushed.pushed_at_unix = None;
        let listing = GitFetchAsk::Listing {
            named_refs: ["refs/heads/".to_string()].into(),
        };
        let transfer = GitFetchAsk::Transfer {
            wants: vec![pushed.head_oid.clone()],
            shallow: false,
        };
        for (ask, request, full_view, expected) in [
            (&listing, &pushed, true, true),
            (&listing, &pushed, false, false),
            (&listing, &unpushed, true, false),
            (&GitFetchAsk::Advertisement, &pushed, true, false),
            (&transfer, &pushed, true, false),
        ] {
            assert_eq!(
                ask.lists_without_preparing(request, full_view),
                expected,
                "{ask:?} {full_view}"
            );
        }
    }

    #[test]
    fn candidates_admit_every_request_a_fetch_includes() {
        let request = open_request();
        let name = canonical_request_ref(&request.name);
        let asks = [
            GitFetchAsk::Advertisement,
            GitFetchAsk::Listing {
                named_refs: BTreeSet::new(),
            },
            GitFetchAsk::Listing {
                named_refs: [name.clone()].into(),
            },
            GitFetchAsk::Listing {
                named_refs: [name[..name.len() - 2].to_string()].into(),
            },
            GitFetchAsk::Listing {
                named_refs: ["refs/".to_string(), "refs/tags/".to_string()].into(),
            },
            GitFetchAsk::Listing {
                named_refs: ["refs/heads/".to_string()].into(),
            },
            GitFetchAsk::Listing {
                named_refs: ["refs/heads/other".to_string(), "refs/tags/".to_string()].into(),
            },
            GitFetchAsk::Listing {
                named_refs: ["refs/heads/other".to_string(), "refs/remotes/".to_string()].into(),
            },
            GitFetchAsk::Transfer {
                wants: vec![request.head_oid.clone()],
                shallow: false,
            },
            GitFetchAsk::Transfer {
                wants: vec!["other".to_string()],
                shallow: false,
            },
            GitFetchAsk::Transfer {
                wants: vec!["other".to_string()],
                shallow: true,
            },
        ];
        let mut narrowed = 0;
        for ask in &asks {
            for (user, invitee, own) in [
                (Some("author"), false, true),
                (Some("reader"), true, true),
                (Some("reader"), false, false),
                (None, false, false),
            ] {
                let admitted = ask.candidates().admits(&request, own);
                assert!(
                    admitted || !ask.includes(&request, user, invitee),
                    "{ask:?} {user:?} {invitee}"
                );
                narrowed += usize::from(!admitted);
            }
        }
        assert!(narrowed > 0);
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
