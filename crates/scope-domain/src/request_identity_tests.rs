use super::{
    requests::*,
    requests_tests::{open_request, working_request},
};

#[test]
fn identity_edit_supports_each_field_combination_and_rejects_empty_or_unchanged_inputs() {
    let request = working_request();
    let original_description = request.description_markdown.clone();

    let title_only = edit_request_identity(
        request,
        false,
        EditRequestIdentityInput {
            title: Some("Focused title".to_string()),
            now_unix: 20,
            ..identity_input()
        },
    )
    .unwrap();
    assert_eq!(title_only.request.title, "Focused title");
    assert_eq!(
        title_only.request.description_markdown,
        original_description
    );
    assert_eq!(title_only.event.kind, RequestEventKind::IdentityEdited);

    let description_only = edit_request_identity(
        title_only.request,
        false,
        EditRequestIdentityInput {
            description_markdown: Some("Focused description".to_string()),
            expected_description_markdown: Some(original_description.clone()),
            now_unix: 21,
            ..identity_input()
        },
    )
    .unwrap();
    assert_eq!(description_only.request.title, "Focused title");
    assert_eq!(
        description_only.request.description_markdown,
        "Focused description"
    );

    let empty = edit_request_identity(
        description_only.request.clone(),
        false,
        EditRequestIdentityInput {
            now_unix: 22,
            ..identity_input()
        },
    )
    .unwrap_err();
    assert_eq!(empty.kind, crate::error::DomainErrorKind::InvalidInput);

    let unchanged = edit_request_identity(
        description_only.request,
        false,
        EditRequestIdentityInput {
            title: Some("Focused title".to_string()),
            description_markdown: Some("Focused description".to_string()),
            expected_description_markdown: Some("Focused description".to_string()),
            now_unix: 22,
            ..identity_input()
        },
    )
    .unwrap_err();
    assert_eq!(unchanged.kind, crate::error::DomainErrorKind::Conflict);
}

#[test]
fn open_request_identity_edits_preserve_submission() {
    let request = open_request();
    let mutation = edit_request_identity(
        request.clone(),
        false,
        EditRequestIdentityInput {
            description_markdown: Some("Changed while open".to_string()),
            now_unix: 22,
            ..identity_input()
        },
    )
    .unwrap();
    assert_eq!(mutation.request.state(), RequestState::Open);
    assert_eq!(mutation.request.submitted_at_unix, Some(20));
    assert_eq!(mutation.request.description_markdown, "Changed while open");
}

#[test]
fn description_edit_rejects_a_stale_expected_value() {
    let error = edit_request_identity(
        working_request(),
        false,
        EditRequestIdentityInput {
            description_markdown: Some("new description".to_string()),
            expected_description_markdown: Some("stale description".to_string()),
            now_unix: 11,
            ..identity_input()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, crate::error::DomainErrorKind::Conflict);
}

#[test]
fn identity_event_collision_precedes_edit_authorization() {
    let error = edit_request_identity(
        working_request(),
        true,
        EditRequestIdentityInput {
            actor_user_id: "outsider".to_string(),
            actor_can_edit_identity: false,
            event_id: "existing_event".to_string(),
            title: Some("Changed".to_string()),
            now_unix: 20,
            ..identity_input()
        },
    )
    .unwrap_err();
    assert_eq!(error.message, "request event already exists");
}

fn identity_input() -> EditRequestIdentityInput {
    EditRequestIdentityInput {
        request_id: "request_1".to_string(),
        actor_user_id: "author".to_string(),
        actor_can_edit_identity: true,
        event_id: "event_identity".to_string(),
        title: None,
        description_markdown: None,
        expected_description_markdown: None,
        now_unix: 22,
    }
}
