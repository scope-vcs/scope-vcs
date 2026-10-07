use super::{
    Request, RequestActorRole, StartRequestFacts, StartRequestInput, SubmitRequestInput,
    start_request, submit_request,
};
use crate::content::{DEFAULT_GIT_FILE_MODE, SourceBlob};
use crate::views::{ViewDefinition, ViewId, ViewIncludes, ViewReaders, Views};

pub(crate) fn source_blob(git_oid: &str) -> SourceBlob {
    SourceBlob {
        content_ref: crate::content_ref::ContentRef::blob_sha256(git_oid),
        sha256: format!("sha256-{git_oid}"),
        git_oid: git_oid.to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    }
}

pub(crate) fn agent() -> ViewId {
    ViewId::parse("agent").unwrap()
}

pub(crate) fn views_with_agent() -> Views {
    let mut definitions = Vec::<ViewDefinition>::from(Views::builtin());
    definitions.push(ViewDefinition {
        id: agent(),
        name: "Agent".into(),
        includes: ViewIncludes::Some([ViewId::public()].into()),
        readers: ViewReaders::Assigned,
    });
    Views::new(definitions).unwrap()
}

pub(crate) fn start_input(author_role: RequestActorRole) -> StartRequestInput {
    StartRequestInput {
        id: "request_1".to_string(),
        repo_id: "owner/repo".to_string(),
        name: "fix-parser".to_string(),
        author_user_id: "author".to_string(),
        title: Some("Fix parser".to_string()),
        author_role,
        author_view: ViewId::public(),
        view: ViewId::public(),
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
        &Views::builtin(),
    )
    .unwrap()
    .request
}

pub(crate) fn pushed_draft(author_role: RequestActorRole) -> Request {
    let mut request = start_request(
        StartRequestFacts::default(),
        start_input(author_role),
        &Views::builtin(),
    )
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
