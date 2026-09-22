use scope_domain::{
    content::SourceBlob,
    projection::LogicalCommitOrigin,
    repository::{
        RepoLifecycleState,
        collaboration::{RepositoryInvite, RepositoryMemberPermissions},
    },
    runs::{
        attempt::AttemptState,
        job::RunJobState,
        run::RunState,
        source::RunSource,
        step::{AttemptTerminalReason, StepState},
        workflow::{
            definition::CompiledWorkflow,
            identity::{WorkflowIdentity, WorkflowPath},
            revision::WorkflowRevision,
        },
    },
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

const SHA256: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const GIT_OID: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn persisted_roundtrip<T: DeserializeOwned + Serialize>(fixture: Value) -> T {
    let value: T = serde_json::from_value(fixture.clone()).unwrap();
    assert_eq!(serde_json::to_value(&value).unwrap(), fixture);
    value
}

#[test]
fn persisted_domain_shapes_roundtrip_the_recorded_json() {
    persisted_roundtrip::<SourceBlob>(json!({
        "content_ref": { "BlobSha256": SHA256 },
        "sha256": SHA256,
        "git_oid": GIT_OID,
        "git_file_mode": "100644",
        "size_bytes": 42,
    }));
    persisted_roundtrip::<LogicalCommitOrigin>(json!({
        "PublicRequestMerge": {
            "request_id": "request-7",
            "public_base_oid": "1111111111111111111111111111111111111111",
            "public_parent_oids": ["2222222222222222222222222222222222222222"],
            "request_head_oid": "3333333333333333333333333333333333333333",
            "commits": [{
                "oid": "4444444444444444444444444444444444444444",
                "parent_oids": ["2222222222222222222222222222222222222222"],
                "tree_oid": "5555555555555555555555555555555555555555",
                "changed_paths": ["/src/lib.rs"],
            }],
            "preserve_public_commits": true,
        }
    }));
    persisted_roundtrip::<RepoLifecycleState>(json!("AwaitingFirstPush"));
    persisted_roundtrip::<RepositoryMemberPermissions>(json!({
        "can_push": true,
        "can_change_file_visibility": false,
    }));
    persisted_roundtrip::<RepositoryInvite>(json!({
        "id": "invite-1",
        "repo_id": "owner/repo",
        "invited_email": "Maintainer@example.com",
        "invited_email_normalized": "maintainer@example.com",
        "permissions": {
            "can_push": true,
            "can_change_file_visibility": false,
        },
        "invited_by_user_id": "owner-user",
        "link_hashes": ["link-hash"],
        "created_at_unix": 100,
        "updated_at_unix": 101,
        "expires_at_unix": 200,
        "accepted_by_user_id": null,
        "accepted_at_unix": null,
        "revoked_at_unix": null,
    }));
    persisted_roundtrip::<RunSource>(json!({
        "kind": "ephemeral-git-bundle",
        "object": {
            "content_ref": { "GitBundleSha256": SHA256 },
            "sha256": SHA256,
            "git_oid": GIT_OID,
            "git_file_mode": "100644",
            "size_bytes": 42,
        }
    }));
    persisted_roundtrip::<AttemptTerminalReason>(json!({
        "kind": "runtime-setup-failed",
        "exit_code": 127,
        "message": "runtime unavailable",
    }));
    persisted_roundtrip::<AttemptTerminalReason>(json!({
        "kind": "provider-capacity-rejected",
        "message": "provider full",
    }));
}

#[test]
fn compiled_workflow_persisted_json_and_revision_digest_are_stable() {
    let definition = persisted_roundtrip::<CompiledWorkflow>(json!({
        "name": "Checks",
        "triggers": {
            "manual": true,
            "push_main": true,
        },
        "jobs": [{
            "id": "build",
            "needs": [],
            "container": {
                "image": format!("rust@sha256:{SHA256}"),
            },
            "timeout_seconds": 600,
            "caches": [],
            "environment": {
                "RUST_BACKTRACE": "1",
            },
            "steps": [{
                "name": "Build",
                "run": "cargo build",
            }],
        }],
    }));
    let revision = WorkflowRevision::new(
        WorkflowIdentity::new(
            "owner/repo",
            WorkflowPath::parse("/.scope/runs/checks.yml").unwrap(),
        )
        .unwrap(),
        definition,
    )
    .unwrap();
    assert_eq!(
        revision.digest(),
        "0740cd8887d731cfb569b266b9228cba9edec98aff5780a0f9ee006815f62699"
    );
}

/// `as_str` is what SQL predicates and schema constraints are built from, so it
/// must stay the single persisted spelling of every run state variant.
#[test]
fn run_state_names_match_their_persisted_encoding() {
    fn assert_persisted_names<T: Serialize + Copy>(variants: &[T], names: &[&str]) {
        let encoded = variants
            .iter()
            .map(|variant| match serde_json::to_value(variant).unwrap() {
                Value::String(value) => value,
                other => panic!("run state must serialize to a string, found {other}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(encoded, names);
    }

    assert_persisted_names(&RunState::ALL, &RunState::ALL.map(RunState::as_str));
    assert_persisted_names(
        &RunJobState::ALL,
        &RunJobState::ALL.map(RunJobState::as_str),
    );
    assert_persisted_names(
        &AttemptState::ALL,
        &AttemptState::ALL.map(AttemptState::as_str),
    );
    assert_persisted_names(&StepState::ALL, &StepState::ALL.map(StepState::as_str));
}
