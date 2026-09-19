//! Actor identity and mutation intent. Locked persistence facts supply domain capabilities.

#[derive(Clone, Debug)]
pub struct SubmitRequestCommand {
    pub request_id: String,
    pub actor_user_id: String,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct CloseRequestCommand {
    pub request_id: String,
    pub actor_user_id: String,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct EditRequestIdentityCommand {
    pub request_id: String,
    pub actor_user_id: String,
    pub event_id: String,
    pub title: Option<String>,
    pub description_markdown: Option<String>,
    pub expected_description_markdown: Option<String>,
    pub now_unix: u64,
}

/// A request whose head a committed main push already carries.
#[derive(Clone, Debug)]
pub struct CompleteLandedRequestCommand {
    pub request_id: String,
    pub actor_user_id: String,
    pub merged_event_id: String,
    pub landed_head_oid: String,
    pub main_oid: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct ExpectedRequestAutoMerge {
    pub intent_id: String,
    pub revision_id: String,
    pub head_oid: String,
    pub claim_token: String,
    pub fulfilled_event_id: String,
}

#[derive(Clone, Debug)]
pub struct MergeRequestContentCommand {
    pub owner: String,
    pub name: String,
    pub request_id: String,
    pub actor_user_id: String,
    pub merged_event_id: String,
    /// The repository state the merge was prepared against; any drift is a conflict.
    pub expected_git_frontier: scope_domain::repository::git::GitFrontier,
    pub expected_repo_change_version: u64,
    pub expected_request_head_oid: String,
    pub expected_auto_merge: Option<ExpectedRequestAutoMerge>,
    pub update: scope_domain::reviewed_updates::content::ReviewedUpdateInput,
    pub landing_file_mutation: scope_domain::landing_file::RepositoryLandingFileMutation,
    pub workflow_catalog: scope_domain::runs::catalog::RepositoryWorkflowCatalog,
    pub origin: scope_domain::repository::updates::RequestMergeOrigin,
    pub now_unix: u64,
}
