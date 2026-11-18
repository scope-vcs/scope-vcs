use super::{
    MergeGateAuthority, MergeGateInput, MergeRequestInput, MergeRequestLanding, Request,
    RequestActor, RequestAutoMergeIntent, RequestAutoMergeMutation, RequestAutoMergeReadiness,
    RequestAutoMergeStopReason, RequestChecksOutcome, RequestLifecycleMutation,
    RequestMergeSubject, RequestReviewsOutcome, RequestRevision, RequestViewMains, RequestViewer,
    evaluate_merge_gate, fulfill_request_auto_merge, merge_request, request_auto_merge_event_id,
    request_auto_merge_readiness, stop_request_auto_merge,
};
use crate::{
    error::DomainError,
    repository::{RepoLifecycleState, access::RepositoryActor, git::GitFrontier},
};

#[derive(Clone, Copy, Debug)]
pub struct MergeFacts<'a> {
    pub request: &'a Request,
    pub repository_id: &'a str,
    pub repository_lifecycle: RepoLifecycleState,
    pub actor: RequestActor<'a>,
    pub actor_user_id: &'a str,
}

#[derive(Clone, Copy, Debug)]
pub struct MergeReadinessFacts<'a> {
    pub view_mains: &'a RequestViewMains,
    pub checks: RequestChecksOutcome,
    pub reviews: RequestReviewsOutcome,
}

#[derive(Clone, Copy, Debug)]
pub struct StoredAutoMergeClaim<'a> {
    pub intent: &'a RequestAutoMergeIntent,
    pub claim_token: Option<&'a str>,
    pub claim_expires_at_unix: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
pub struct ExpectedAutoMergeClaim<'a> {
    pub revision_id: &'a str,
    pub head_oid: &'a str,
    pub claim_token: &'a str,
    pub fulfilled_event_id: &'a str,
}

#[derive(Clone, Copy, Debug)]
pub enum MergeAutoMerge<'a> {
    None,
    Active(&'a RequestAutoMergeIntent),
    Claimed {
        expected: ExpectedAutoMergeClaim<'a>,
        stored: Option<StoredAutoMergeClaim<'a>>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeGitHead {
    Missing,
    Moved,
    AsPrepared,
}

impl MergeGitHead {
    pub fn observed(current: Option<&GitFrontier>, prepared: &GitFrontier) -> Self {
        match current {
            None => Self::Missing,
            Some(current) if current == prepared => Self::AsPrepared,
            Some(_) => Self::Moved,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MergeCommitFacts<'a> {
    pub latest_revision: Option<&'a RequestRevision>,
    pub auto_merge: MergeAutoMerge<'a>,
    pub actor_exists: bool,
    pub repository_incarnation_id: &'a str,
    pub repository_change_version: u64,
    pub git_head: MergeGitHead,
    pub readiness: Option<MergeReadinessFacts<'a>>,
    pub prepared_request_head_oid: &'a str,
    pub prepared_repository_change_version: u64,
    pub merged_main_oid: &'a str,
    pub merged_event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub enum MergeStep<'a> {
    Authorize { expected_head_oid: Option<&'a str> },
    Clear(MergeReadinessFacts<'a>),
    Commit(Box<MergeCommitFacts<'a>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeDecision {
    Proceed,
    Merge(Box<MergeOutcome>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeOutcome {
    pub request: RequestLifecycleMutation,
    auto_merge: Option<AutoMergeFulfillment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AutoMergeFulfillment {
    intent: RequestAutoMergeIntent,
    event_id: Option<String>,
}

impl MergeOutcome {
    pub fn fulfill_auto_merge(
        &self,
        main_oid: &str,
        now_unix: u64,
    ) -> Result<Option<RequestAutoMergeMutation>, DomainError> {
        self.auto_merge
            .as_ref()
            .map(|fulfillment| {
                fulfill_request_auto_merge(
                    &self.request.request,
                    &fulfillment.intent,
                    main_oid.to_string(),
                    fulfillment.event_id.clone().unwrap_or_else(|| {
                        request_auto_merge_event_id("fulfilled", &fulfillment.intent.id)
                    }),
                    now_unix,
                )
            })
            .transpose()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeRefusal {
    Rejected(DomainError),
    RepositoryMoved,
    ReadinessUnavailable,
    AutoMergeStopped {
        error: DomainError,
        stop: Box<RequestAutoMergeMutation>,
    },
}

impl From<DomainError> for MergeRefusal {
    fn from(error: DomainError) -> Self {
        Self::Rejected(error)
    }
}

pub fn decide_merge(
    facts: MergeFacts<'_>,
    step: MergeStep<'_>,
) -> Result<MergeDecision, MergeRefusal> {
    match step {
        MergeStep::Authorize { expected_head_oid } => {
            authorize(facts, expected_head_oid)?;
            Ok(MergeDecision::Proceed)
        }
        MergeStep::Clear(readiness) => {
            ensure_gate_open(facts, readiness)?;
            Ok(MergeDecision::Proceed)
        }
        MergeStep::Commit(commit) => commit_merge(facts, *commit)
            .map(Box::new)
            .map(MergeDecision::Merge),
    }
}

fn authorize(facts: MergeFacts<'_>, expected_head_oid: Option<&str>) -> Result<(), DomainError> {
    let policy = facts.actor.policy(facts.request, facts.actor_user_id);
    if facts.request.repo_id != facts.repository_id || !policy.exact_visible {
        return Err(DomainError::not_found("request not found"));
    }
    if !policy.permissions.can_merge {
        return Err(
            if matches!(facts.actor.access.actor, RepositoryActor::Public) {
                DomainError::forbidden("repo maintainer required")
            } else {
                DomainError::conflict("request cannot be merged")
            },
        );
    }
    if expected_head_oid.is_some_and(|expected| expected != facts.request.head_oid) {
        return Err(DomainError::conflict(
            "request has a new revision; review it before merging",
        ));
    }
    Ok(())
}

fn gate(facts: MergeFacts<'_>, readiness: MergeReadinessFacts<'_>) -> super::MergeGateDecision {
    evaluate_merge_gate(MergeGateInput::RequestMerge {
        repository_lifecycle: facts.repository_lifecycle,
        subject: RequestMergeSubject::new(facts.request, readiness.view_mains),
        viewer: &RequestViewer::new(
            facts.actor.access.clone(),
            Some(facts.actor_user_id),
            facts.actor.is_invitee,
        ),
        views: facts.actor.views,
        checks: readiness.checks,
        reviews: readiness.reviews,
    })
}

fn ensure_gate_open(
    facts: MergeFacts<'_>,
    readiness: MergeReadinessFacts<'_>,
) -> Result<(), DomainError> {
    gate_refusal(facts, gate(facts, readiness)).map_or(Ok(()), Err)
}

fn gate_refusal(facts: MergeFacts<'_>, gate: super::MergeGateDecision) -> Option<DomainError> {
    if gate.authority() == Some(MergeGateAuthority::RequestMerge) {
        return None;
    }
    let message = gate.refusal().unwrap_or("request cannot be merged");
    Some(if facts.actor.access.is_maintainer() {
        DomainError::conflict(message)
    } else {
        DomainError::forbidden(message)
    })
}

fn commit_merge(
    facts: MergeFacts<'_>,
    commit: MergeCommitFacts<'_>,
) -> Result<MergeOutcome, MergeRefusal> {
    let request = facts.request;
    let (intent, claim) = match commit.auto_merge {
        MergeAutoMerge::None => (None, None),
        MergeAutoMerge::Active(intent) => (Some(intent), None),
        MergeAutoMerge::Claimed { expected, stored } => {
            let intent = stored
                .filter(|stored| {
                    stored.intent.is_active()
                        && stored.intent.request_id == request.id
                        && stored.intent.repo_id == facts.repository_id
                        && stored.intent.revision_id == expected.revision_id
                        && stored.intent.head_oid == expected.head_oid
                        && stored.claim_token == Some(expected.claim_token)
                        && stored
                            .claim_expires_at_unix
                            .is_some_and(|expires| expires >= commit.now_unix)
                })
                .map(|stored| stored.intent)
                .ok_or_else(|| DomainError::conflict("auto-merge claim is no longer current"))?;
            (Some(intent), Some(expected))
        }
    };
    let stop = |reason, error: DomainError| -> Result<MergeRefusal, DomainError> {
        let Some(intent) = intent else {
            return Ok(MergeRefusal::Rejected(error));
        };
        Ok(MergeRefusal::AutoMergeStopped {
            error,
            stop: Box::new(stop_request_auto_merge(
                request,
                intent,
                reason,
                request_auto_merge_event_id("stopped", &intent.id),
                commit.now_unix,
            )?),
        })
    };

    let authorized_revision_is_current = intent.is_none_or(|intent| {
        commit.latest_revision.is_some_and(|revision| {
            revision.id == intent.revision_id
                && revision.new_head_oid == intent.head_oid
                && request.head_oid == intent.head_oid
        })
    });
    if request.head_oid != commit.prepared_request_head_oid || !authorized_revision_is_current {
        let changed =
            DomainError::conflict("request changed since merge was prepared; retry merge");
        return Err(if claim.is_some() || !authorized_revision_is_current {
            stop(RequestAutoMergeStopReason::RequestChanged, changed)?
        } else {
            MergeRefusal::Rejected(changed)
        });
    }
    if !commit.actor_exists {
        return Err(DomainError::not_found("user not found").into());
    }
    if intent
        .is_some_and(|intent| intent.repository_incarnation_id != commit.repository_incarnation_id)
    {
        return Err(DomainError::conflict("auto-merge repository incarnation changed").into());
    }
    let readiness = commit.readiness.ok_or(MergeRefusal::ReadinessUnavailable)?;
    let gate = gate(facts, readiness);
    if claim.is_some()
        && let Some(intent) = intent
    {
        if intent.actor_user_id != facts.actor_user_id {
            return Err(
                DomainError::conflict("auto-merge actor does not match the authorization").into(),
            );
        }
        if !facts.actor.access.is_maintainer() {
            return Err(stop(
                RequestAutoMergeStopReason::AccessRevoked,
                DomainError::forbidden("repo maintainer required"),
            )?);
        }
        match request_auto_merge_readiness(gate) {
            RequestAutoMergeReadiness::Ready => {}
            RequestAutoMergeReadiness::Waiting(_) => {
                return Err(DomainError::conflict("auto-merge CI is not ready").into());
            }
            RequestAutoMergeReadiness::Stop(reason) => {
                return Err(stop(reason, DomainError::conflict(reason.message()))?);
            }
        }
    }
    if commit.repository_change_version != commit.prepared_repository_change_version {
        return Err(MergeRefusal::RepositoryMoved);
    }
    match commit.git_head {
        MergeGitHead::Missing => {
            return Err(DomainError::conflict("repo has no accepted Git head").into());
        }
        MergeGitHead::Moved => return Err(MergeRefusal::RepositoryMoved),
        MergeGitHead::AsPrepared => {}
    }
    if let Some(refusal) = gate_refusal(facts, gate) {
        return Err(refusal.into());
    }
    let mutation = merge_request(
        request,
        MergeRequestInput {
            request_id: request.id.clone(),
            actor_user_id: facts.actor_user_id.to_string(),
            actor_is_maintainer: facts.actor.access.is_maintainer(),
            landing: MergeRequestLanding::NativeMerge,
            merged_head_oid: commit.prepared_request_head_oid.to_string(),
            merged_main_oid: commit.merged_main_oid.to_string(),
            merged_event_id: commit.merged_event_id,
            now_unix: commit.now_unix,
        },
    )?;
    Ok(MergeOutcome {
        request: mutation,
        auto_merge: intent.map(|intent| AutoMergeFulfillment {
            intent: intent.clone(),
            event_id: claim.map(|claim| claim.fulfilled_event_id.to_string()),
        }),
    })
}
