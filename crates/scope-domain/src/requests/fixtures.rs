use super::{
    Request, RequestActorRole, RequestAudience, StartRequestFacts, StartRequestInput,
    SubmitRequestInput, start_request, submit_request,
};
use crate::content::{DEFAULT_GIT_FILE_MODE, SourceBlob};

pub(crate) fn source_blob(git_oid: &str) -> SourceBlob {
    SourceBlob {
        content_ref: crate::content_ref::ContentRef::blob_sha256(git_oid),
        sha256: format!("sha256-{git_oid}"),
        git_oid: git_oid.to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    }
}

pub(crate) fn start_input(author_role: RequestActorRole) -> StartRequestInput {
    StartRequestInput {
        id: "request_1".to_string(),
        repo_id: "owner/repo".to_string(),
        name: "fix-parser".to_string(),
        author_user_id: "author".to_string(),
        title: Some("Fix parser".to_string()),
        author_role,
        audience: RequestAudience::Public,
        base_main_oid: "base".to_string(),
        event_id: "event_started".to_string(),
        now_unix: 10,
    }
}

pub(crate) fn submit_input() -> SubmitRequestInput {
    SubmitRequestInput {
        request_id: "request_1".to_string(),
        actor_user_id: "author".to_string(),
        actor_is_author: true,
        actor_can_submit: true,
        event_id: "event_submitted".to_string(),
        now_unix: 20,
    }
}

pub(crate) fn working_request() -> Request {
    start_request(
        StartRequestFacts::default(),
        start_input(RequestActorRole::Public),
    )
    .unwrap()
    .request
}

pub(crate) fn pushed_draft(author_role: RequestActorRole) -> Request {
    let mut request = start_request(StartRequestFacts::default(), start_input(author_role))
        .unwrap()
        .request;
    request.head_oid = "head".to_string();
    request.git_snapshot = Some(source_blob("head"));
    request.updated_at_unix = 11;
    request
}

pub(crate) fn open_request() -> Request {
    submit_request(&pushed_draft(RequestActorRole::Public), submit_input())
        .unwrap()
        .request
}
