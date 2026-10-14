use super::merge_gate::{RequestMergeSubject, RequestMergeability, evaluate_request_merge};
use super::{
    Request, RequestActorRole, RequestState, checks::RequestChecksOutcome,
    lifecycle::ensure_request_close_allowed,
};
use crate::repository::access::{RepositoryAccess, RepositoryActor};
use crate::views::{ViewId, Views};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestViewer<'a> {
    pub access: RepositoryAccess,
    pub user_id: Option<&'a str>,
    pub is_invitee: bool,
}

impl<'a> RequestViewer<'a> {
    pub fn new(access: RepositoryAccess, user_id: Option<&'a str>, is_invitee: bool) -> Self {
        Self {
            access,
            user_id,
            is_invitee: user_id.is_some() && is_invitee,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestPermissions {
    pub can_open_discussion: bool,
    pub can_reply_to_discussion: bool,
    pub can_wait_after_reply: bool,
    pub can_transition_discussion: bool,
    pub can_edit_identity: bool,
    pub can_pull_branch: bool,
    pub can_push_branch: bool,
    pub can_submit: bool,
    pub can_manage_invitees: bool,
    pub can_leave_request: bool,
    pub can_close: bool,
    pub can_merge: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestPolicyDecision {
    pub listable: bool,
    pub exact_visible: bool,
    pub discussion_visible: bool,
    pub activity_stream_visible: bool,
    pub branch_mutable: bool,
    pub counts_as_open: bool,
    pub permissions: RequestPermissions,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestListPredicate<'a> {
    All(Vec<RequestListPredicate<'a>>),
    Any(Vec<RequestListPredicate<'a>>),
    View(ViewId),
    Submitted,
    Author(&'a str),
    Invitee(&'a str),
}

pub fn request_list_predicate<'a>(
    access: RepositoryAccess,
    viewer_user_id: Option<&'a str>,
    views: &Views,
) -> RequestListPredicate<'a> {
    let anyone = views.anyone();
    let mut visible = Vec::new();
    if let Some(anyone) = anyone {
        let mut anyone_access = vec![RequestListPredicate::Submitted];
        if let Some(viewer_user_id) = viewer_user_id {
            anyone_access.push(RequestListPredicate::Author(viewer_user_id));
            anyone_access.push(RequestListPredicate::Invitee(viewer_user_id));
        }
        visible.push(RequestListPredicate::All(vec![
            RequestListPredicate::View(anyone.clone()),
            RequestListPredicate::Any(anyone_access),
        ]));
    }
    if access.is_maintainer() {
        visible.extend(
            views
                .readable_by(Some(&access.view))
                .into_iter()
                .filter(|view| Some(*view) != anyone)
                .map(|view| RequestListPredicate::View(view.clone())),
        );
    }
    RequestListPredicate::Any(visible)
}

impl RequestListPredicate<'_> {
    fn matches(&self, request: &Request, viewer_is_invitee: bool) -> bool {
        match self {
            Self::All(predicates) => predicates
                .iter()
                .all(|predicate| predicate.matches(request, viewer_is_invitee)),
            Self::Any(predicates) => predicates
                .iter()
                .any(|predicate| predicate.matches(request, viewer_is_invitee)),
            Self::View(view) => request.view == *view,
            Self::Submitted => request.is_submitted(),
            Self::Author(viewer_user_id) => request.is_author(viewer_user_id),
            Self::Invitee(_) => viewer_is_invitee,
        }
    }
}

pub fn request_actor_role(access: RepositoryAccess) -> RequestActorRole {
    match access.actor {
        RepositoryActor::Owner => RequestActorRole::Owner,
        RepositoryActor::Member => RequestActorRole::Member,
        RepositoryActor::Public => RequestActorRole::Public,
    }
}

pub fn request_policy(
    request: &Request,
    viewer: RequestViewer<'_>,
    views: &Views,
) -> RequestPolicyDecision {
    let maintainer = viewer.access.is_maintainer();
    let authenticated = viewer.user_id.is_some();
    let author = viewer
        .user_id
        .is_some_and(|user_id| request.is_author(user_id));
    let invitee = viewer.is_invitee;
    let anyone_view = views.anyone() == Some(&request.view);
    let full_view = &request.view == views.full();
    let reads_request_view =
        views.get(&request.view).is_some() && views.may_read(&viewer.access.view, &request.view);
    let merges_into_canonical_main = request_merger(
        &viewer,
        request.author_user_id.as_deref(),
        &request.view,
        views,
    );
    let submitted = request.is_submitted();
    let terminal = request.is_terminal();
    let open = request.state() == RequestState::Open;

    let exact_visible = if anyone_view {
        submitted || author || invitee
    } else {
        maintainer && reads_request_view
    };
    let listable = request_list_predicate(viewer.access.clone(), viewer.user_id, views)
        .matches(request, viewer.is_invitee);
    let branch_actor = author || (anyone_view && invitee) || (maintainer && reads_request_view);
    let branch_mutable = exact_visible && branch_actor && !terminal;
    let discussion_visible = exact_visible;
    let activity_stream_visible = discussion_visible && listable;
    let can_discuss =
        discussion_visible && authenticated && (!full_view || (maintainer && !terminal));

    let permissions = RequestPermissions {
        can_open_discussion: can_discuss,
        can_reply_to_discussion: can_discuss,
        can_wait_after_reply: can_discuss && maintainer && open,
        can_transition_discussion: discussion_visible && authenticated && (!full_view || !terminal),
        can_edit_identity: exact_visible && !terminal && (author || maintainer),
        can_pull_branch: exact_visible,
        can_push_branch: branch_mutable,
        can_submit: exact_visible && !submitted && author,
        can_manage_invitees: exact_visible && anyone_view && !terminal && (author || maintainer),
        can_leave_request: exact_visible && anyone_view && invitee && !terminal,
        can_close: exact_visible
            && viewer.user_id.is_some_and(|user_id| {
                ensure_request_close_allowed(request, user_id, maintainer).is_ok()
            }),
        can_merge: exact_visible && merges_into_canonical_main && open,
    };

    RequestPolicyDecision {
        listable,
        exact_visible,
        discussion_visible,
        activity_stream_visible,
        branch_mutable,
        counts_as_open: open && exact_visible,
        permissions,
    }
}

pub(super) fn request_merger(
    viewer: &RequestViewer<'_>,
    author_user_id: Option<&str>,
    request_view: &ViewId,
    views: &Views,
) -> bool {
    let author = viewer.user_id.is_some() && viewer.user_id == author_user_id;
    viewer.access.is_maintainer()
        && (views.may_read(&viewer.access.view, views.full())
            || (author && viewer.access.can_push && &viewer.access.view == request_view))
}

pub fn request_list_mergeability(
    subject: RequestMergeSubject<'_>,
    viewer: &RequestViewer<'_>,
    views: &Views,
    checks: RequestChecksOutcome,
    repository_lifecycle: crate::repository::RepoLifecycleState,
) -> RequestMergeability {
    evaluate_request_merge(repository_lifecycle, subject, viewer, views, checks)
}

pub fn request_mergeability(
    request: &Request,
    viewer: &RequestViewer<'_>,
    views: &Views,
    checks: RequestChecksOutcome,
    repository_lifecycle: crate::repository::RepoLifecycleState,
) -> RequestMergeability {
    request_list_mergeability(request.into(), viewer, views, checks, repository_lifecycle)
}
