use super::{
    GitHubPullRequest, Request, closes_carrier_pull_request, retains_request_branch_for_carrier,
};
use crate::github::connection::GitHubConnection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndedRequestEffect {
    PurgeInvitees,
    DeleteGitHubBranch,
    CloseCarrierPullRequest,
    RecomputeChildCarrierBases,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeletedDraftEffect {
    RetirePoolPacks,
    DeleteGitHubBranch,
    TombstoneAttachments,
    DeleteRequestRows,
}

pub const DELETED_DRAFT_EFFECTS: [DeletedDraftEffect; 4] = [
    DeletedDraftEffect::RetirePoolPacks,
    DeletedDraftEffect::DeleteGitHubBranch,
    DeletedDraftEffect::TombstoneAttachments,
    DeletedDraftEffect::DeleteRequestRows,
];

#[derive(Clone, Copy, Debug)]
pub struct RequestCarrierFacts<'a> {
    pub connection: Option<&'a GitHubConnection>,
    pub pull_request: Option<&'a GitHubPullRequest>,
}

pub fn ended_request_effects(
    request: &Request,
    carrier: RequestCarrierFacts<'_>,
) -> Vec<EndedRequestEffect> {
    if !request.is_terminal() {
        return Vec::new();
    }
    let mut effects = vec![EndedRequestEffect::PurgeInvitees];
    if !retains_request_branch_for_carrier(request, carrier.connection, carrier.pull_request) {
        effects.push(EndedRequestEffect::DeleteGitHubBranch);
    }
    if closes_carrier_pull_request(request, carrier.connection) {
        effects.push(EndedRequestEffect::CloseCarrierPullRequest);
    }
    effects.push(EndedRequestEffect::RecomputeChildCarrierBases);
    effects
}
