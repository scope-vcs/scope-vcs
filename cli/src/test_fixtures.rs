use scope_api_contract::{
    AccountSessionResponse, RepoLifecycleState, RepoSummaryResponse, RepositoryAccessResponse,
    RepositoryActor, UserResponse,
};
use serde_json::Value;

pub fn session_response(id: &str, handle: &str, email: &str) -> Value {
    serde_json::to_value(AccountSessionResponse {
        identity: None,
        user: Some(UserResponse {
            id: id.into(),
            handle: handle.into(),
            email: email.into(),
            email_verified: true,
        }),
    })
    .unwrap()
}

pub fn repository_response(overrides: Value) -> Value {
    let mut repo = serde_json::to_value(RepoSummaryResponse {
        description: None,
        website_url: None,
        id: "repo_one".into(),
        owner_handle: "owner".into(),
        name: "repo".into(),
        git_remote_url: "https://scope.example/git/public/owner/repo".into(),
        lifecycle_state: RepoLifecycleState::Ready,
        change_version: 1,
        access: RepositoryAccessResponse {
            actor: RepositoryActor::Public,
            can_read_private_files: false,
            can_push: false,
            can_change_file_visibility: false,
            can_apply_changes: false,
            can_manage_members: false,
            can_delete_repo: false,
        },
        open_request_count: 0,
    })
    .unwrap();
    merge(&mut repo, overrides);
    repo
}

fn merge(value: &mut Value, overrides: Value) {
    if let (Some(value), Some(overrides)) = (value.as_object_mut(), overrides.as_object()) {
        for (key, replacement) in overrides {
            merge(
                value.entry(key.clone()).or_insert(Value::Null),
                replacement.clone(),
            );
        }
    } else {
        *value = overrides;
    }
}
