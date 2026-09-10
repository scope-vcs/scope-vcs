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

#[derive(Clone, Debug)]
pub struct MergeRequestContentCommand {
    pub request_id: String,
    pub actor_user_id: String,
    pub merged_event_id: String,
    pub now_unix: u64,
}
