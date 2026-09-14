use scope_domain::{
    content::SourceBlob,
    projection::LogicalCommitOrigin,
    repository::{
        RepoLifecycleState,
        collaboration::{RepositoryInvite, RepositoryMemberPermissions},
    },
    runs::{
        source::RunSource,
        step::AttemptTerminalReason,
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
        "state": "Pending",
        "token_hash": "token-hash",
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
