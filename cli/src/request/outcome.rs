use crate::api::{
    LeaveRequestResponse, RepoSummaryResponse, RequestActivityPageResponse, RequestCloseResponse,
    RequestDiscussionReplyResponse, RequestDiscussionSummaryResponse,
    RequestInviteeMutationResponse, RequestListItemResponse, RequestMutationResponse,
    RequestRatingResponse, RequestSummaryResponse,
};
use serde::Serialize;

pub struct RequestCommandOutcome {
    command: &'static str,
    result: RequestCommandResult,
    human_lines: Vec<String>,
}

impl RequestCommandOutcome {
    pub(super) fn new(
        command: &'static str,
        result: RequestCommandResult,
        human_lines: Vec<String>,
    ) -> Self {
        Self {
            command,
            result,
            human_lines,
        }
    }

    pub fn render(self) -> anyhow::Result<()> {
        crate::execution::emit(self.command, &self.result, self.human_lines)
    }
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum RequestCommandResult {
    Started(StartResult),
    Checkout(CheckoutResult),
    Diff(DiffResult),
    Checks(ChecksResult),
    Detail(DetailResult),
    List(ListResult),
    Mutation(MutationResult),
    Invitee(RepoResponse<RequestInviteeMutationResponse>),
    Leave(TargetResponse<LeaveRequestResponse>),
    Close(TargetResponse<RequestCloseResponse>),
    Discussion(DiscussionResult),
    DiscussionReply(DiscussionReplyResult),
    Rating(TargetResponse<RequestRatingResponse>),
}

#[derive(Serialize)]
pub(super) struct StartResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request: RequestSummaryResponse,
    pub(super) branch: String,
    pub(super) base_oid: String,
    pub(super) remote: String,
}

#[derive(Serialize)]
pub(super) struct DetailResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request: RequestSummaryResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) activity: Option<RequestActivityPageResponse>,
}

#[derive(Serialize)]
pub(super) struct ListResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) requests: Vec<RequestListItemResponse>,
}

#[derive(Serialize)]
pub(super) struct RepoResponse<T> {
    pub(super) repo: RepoSummaryResponse,
    pub(super) response: T,
}

#[derive(Serialize)]
pub(super) struct TargetResponse<T> {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request_id: String,
    pub(super) response: T,
}

#[derive(Serialize)]
pub(super) struct DiscussionResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request_id: String,
    pub(super) discussion: RequestDiscussionSummaryResponse,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) attachments: Vec<scope_api_contract::attachments::RequestAttachmentResponse>,
}

#[derive(Serialize)]
pub(super) struct DiscussionReplyResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request_id: String,
    pub(super) discussion: RequestDiscussionSummaryResponse,
    pub(super) reply: RequestDiscussionReplyResponse,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) attachments: Vec<scope_api_contract::attachments::RequestAttachmentResponse>,
}

#[derive(Serialize)]
pub(super) struct MutationResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) response: RequestMutationResponse,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) attachments: Vec<scope_api_contract::attachments::RequestAttachmentResponse>,
}

#[derive(Serialize)]
pub(super) struct CheckoutResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request: RequestSummaryResponse,
    pub(super) branch: String,
    pub(super) head_oid: String,
}

#[derive(Serialize)]
pub(super) struct DiffResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request_id: String,
    pub(super) revisions: scope_api_contract::RequestRevisionListResponse,
    pub(super) files: Vec<InspectedFile>,
}

#[derive(Serialize)]
pub(super) struct InspectedFile {
    pub(super) commit_oid: String,
    pub(super) diff: scope_api_contract::ReviewFileDiffResponse,
}

#[derive(Serialize)]
pub(super) struct ChecksResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request_id: String,
    pub(super) head_oid: String,
    pub(super) mergeability: scope_api_contract::RequestMergeabilityResponse,
    pub(super) workflow_runs_available: bool,
    pub(super) runs: Vec<scope_api_contract::RepositoryRunSummaryResponse>,
}
