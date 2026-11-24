use super::{
    REQUEST_STACK_MAX_ANCESTORS, RequestBase, RequestChangeSubject, RequestStackStatus,
    RequestState, RequestViewer, request_change_visible,
};
use crate::views::Views;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestStackNode {
    pub id: String,
    pub name: String,
    pub title: String,
    pub state: RequestState,
    pub stack_status: RequestStackStatus,
    pub stack_depth: u32,
    pub head_oid: String,
    pub current_revision_id: Option<String>,
}

impl RequestStackNode {
    pub fn seen_by(
        mut self,
        change: RequestChangeSubject<'_>,
        main_ancestor_oid: &str,
        viewer: &RequestViewer<'_>,
        views: &Views,
    ) -> Self {
        if !request_change_visible(change, viewer, views) {
            self.head_oid = main_ancestor_oid.to_string();
            self.current_revision_id = None;
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestStackParent {
    Available { request: RequestStackNode },
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestStackNavigation {
    pub parent: Option<RequestStackParent>,
    pub ancestors: Vec<RequestStackNode>,
    pub ancestors_unavailable: bool,
}

pub fn request_stack_navigation(
    request_id: &str,
    base: &RequestBase,
    authorized_ancestors: &BTreeMap<String, (RequestStackNode, RequestBase)>,
) -> RequestStackNavigation {
    let parent = base.parent_request_id().map(|id| {
        authorized_ancestors
            .get(id)
            .map_or(RequestStackParent::Unavailable, |(request, _)| {
                RequestStackParent::Available {
                    request: request.clone(),
                }
            })
    });
    let mut ancestors = Vec::new();
    let mut seen = BTreeSet::from([request_id.to_string()]);
    let mut next = base.parent_request_id();
    let mut unavailable = false;
    while let Some(id) = next {
        if ancestors.len() == REQUEST_STACK_MAX_ANCESTORS as usize || !seen.insert(id.to_string()) {
            unavailable = true;
            break;
        }
        let Some((node, base)) = authorized_ancestors.get(id) else {
            unavailable = true;
            break;
        };
        ancestors.push(node.clone());
        next = base.parent_request_id();
    }
    ancestors.reverse();
    RequestStackNavigation {
        parent,
        ancestors,
        ancestors_unavailable: unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str) -> RequestStackNode {
        RequestStackNode {
            id: id.into(),
            name: format!("name-{id}"),
            title: format!("title-{id}"),
            state: RequestState::Open,
            stack_status: RequestStackStatus::WaitingForParent,
            stack_depth: 2,
            head_oid: "head".into(),
            current_revision_id: Some("revision".into()),
        }
    }

    fn parent(id: &str) -> RequestBase {
        RequestBase::Request {
            request_id: id.into(),
            revision_id: "pin".into(),
            head_oid: "pin-head".into(),
        }
    }

    #[test]
    fn a_stack_node_hides_the_head_and_revision_of_a_change_its_viewer_cannot_read() {
        use crate::{
            repository::access::RepositoryAccess, requests::fixtures::actor_views, views::ViewId,
        };
        let view = ViewId::public();
        let draft = RequestChangeSubject {
            view: &view,
            author_user_id: Some("priya"),
            submitted: false,
        };
        for (user_id, submitted, reads_change) in [
            (Some("priya"), false, true),
            (Some("visitor"), false, false),
            (None, false, false),
            (None, true, true),
        ] {
            let viewer = RequestViewer::new(RepositoryAccess::public(), user_id, false);
            let seen = node("draft").seen_by(
                RequestChangeSubject { submitted, ..draft },
                "main",
                &viewer,
                actor_views(),
            );
            assert_eq!(
                (seen.head_oid.as_str(), seen.current_revision_id.as_deref()),
                if reads_change {
                    ("head", Some("revision"))
                } else {
                    ("main", None)
                },
                "{user_id:?} submitted={submitted}"
            );
        }
    }

    #[test]
    fn navigation_stops_at_opaque_edges_and_bounds_cycles_and_depth() {
        let mut nodes = BTreeMap::from([
            ("visible".into(), (node("visible"), parent("hidden"))),
            (
                "grandparent".into(),
                (
                    node("grandparent"),
                    RequestBase::Main {
                        review_base_oid: "main".into(),
                    },
                ),
            ),
        ]);
        let context = request_stack_navigation("child", &parent("visible"), &nodes);
        assert_eq!(
            context
                .ancestors
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["visible"]
        );
        assert!(context.ancestors_unavailable);
        let hidden = request_stack_navigation("child", &parent("hidden"), &nodes);
        assert_eq!(hidden.parent, Some(RequestStackParent::Unavailable));
        assert!(hidden.ancestors.is_empty());
        assert!(hidden.ancestors_unavailable);
        nodes.insert("visible".into(), (node("visible"), parent("child")));
        let cycle = request_stack_navigation("child", &parent("visible"), &nodes);
        assert_eq!(cycle.ancestors.len(), 1);
        assert!(cycle.ancestors_unavailable);
        let nodes = (0..=REQUEST_STACK_MAX_ANCESTORS)
            .map(|index| {
                let id = index.to_string();
                (id.clone(), (node(&id), parent(&(index + 1).to_string())))
            })
            .collect();
        let bounded = request_stack_navigation("child", &parent("0"), &nodes);
        assert_eq!(
            bounded.ancestors.len(),
            REQUEST_STACK_MAX_ANCESTORS as usize
        );
        assert!(bounded.ancestors_unavailable);
    }
}
