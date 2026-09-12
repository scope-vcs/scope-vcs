use super::*;
use serde_json::json;

const IMAGE: &str =
    "ghcr.io/scope/checks@sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const REGISTRY_SECRET_ARN: &str =
    "arn:aws:secretsmanager:us-east-1:123456789012:secret:scope/registry-AbCdEf";

#[test]
fn task_family_is_unique_to_the_attempt() {
    assert_eq!(
        task_family("attempt_123").unwrap(),
        "scope-runner-attempt_123"
    );
}

#[test]
fn task_family_rejects_unsafe_attempt_ids() {
    assert!(task_family("").is_err());
    assert!(task_family("attempt/unsafe").is_err());
}

#[test]
fn secret_names_are_unpredictable_to_the_aws_dispatcher_identity() {
    let cluster = "arn:aws:ecs:us-east-1:123456789012:cluster/scope";
    let first = secret_name(cluster, "attempt_123", &[7; 32]).unwrap();
    let second = secret_name(cluster, "attempt_123", &[8; 32]).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        first,
        secret_name(cluster, "attempt_123", &[7; 32]).unwrap()
    );
    assert!(first.starts_with("scope-vcs/scope/attempts/attempt_123-"));
    assert_eq!(first.rsplit_once('-').unwrap().1.len(), 32);
}

#[test]
fn images_must_remain_digest_pinned() {
    assert!(image_digest("ghcr.io/scope/checks:latest").is_err());
    assert!(image_digest("ghcr.io/scope/checks@sha256:abcd").is_err());
}

#[test]
fn task_definition_arn_must_match_the_exact_family() {
    assert_eq!(
        task_definition_family(
            "arn:aws:ecs:us-east-1:123456789012:task-definition/scope-runner-attempt_abcd:7"
        ),
        Some("scope-runner-attempt_abcd")
    );
    assert_eq!(task_definition_family("not-an-arn"), None);
}

#[tokio::test]
async fn start_registers_a_pinned_fargate_task_and_launches_it_under_the_attempt_id() {
    for registry_credentials in [None, Some(REGISTRY_SECRET_ARN)] {
        let provider = fake::FakeEcs::with_registry_credentials(registry_credentials).await;
        provider.starts.add_permits(1);

        let external_run_id = provider
            .client
            .start(IMAGE, "attempt_1", "scope_bootstrap_test", 86_400)
            .await
            .unwrap();

        assert_eq!(external_run_id, "task-attempt_1");
        let definition = provider.request_body("RegisterTaskDefinition");
        assert_eq!(definition["family"], "scope-runner-attempt_1");
        assert_eq!(definition["networkMode"], "awsvpc");
        assert_eq!(definition["requiresCompatibilities"], json!(["FARGATE"]));
        assert_eq!(
            definition["executionRoleArn"],
            "arn:aws:iam::123456789012:role/test"
        );
        assert_eq!(
            definition["tags"]
                .as_array()
                .unwrap()
                .iter()
                .find(|tag| tag["key"] == "ImageDigest")
                .unwrap()["value"],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        let container = &definition["containerDefinitions"][0];
        assert_eq!(container["name"], "scope-runner");
        assert_eq!(container["image"], IMAGE);
        assert_eq!(
            container["entryPoint"],
            json!(["/scope/bin/scope-runner-runtime"])
        );
        assert_eq!(
            container["secrets"],
            json!([{
                "name": "SCOPE_BOOTSTRAP_TOKEN",
                "valueFrom": "arn:aws:secretsmanager:us-east-1:123456789012:secret:test"
            }])
        );
        assert_eq!(
            container["logConfiguration"]["options"]["awslogs-group"],
            "/scope/test"
        );
        assert_eq!(
            container["repositoryCredentials"]["credentialsParameter"],
            registry_credentials.map_or(serde_json::Value::Null, serde_json::Value::from)
        );

        let run = provider.request_body("RunTask");
        assert_eq!(
            run["cluster"],
            "arn:aws:ecs:us-east-1:123456789012:cluster/test"
        );
        assert_eq!(run["launchType"], "FARGATE");
        assert_eq!(run["count"], 1);
        assert_eq!(run["clientToken"], "attempt_1");
        assert_eq!(run["startedBy"], "attempt_1");
        assert_eq!(
            run["networkConfiguration"]["awsvpcConfiguration"],
            json!({
                "assignPublicIp": "ENABLED",
                "subnets": ["subnet-test"],
                "securityGroups": ["sg-test"]
            })
        );
        assert_eq!(
            run["overrides"]["containerOverrides"][0]["environment"],
            json!([
                {"name": "SCOPE_API_URL", "value": "https://scope.test"},
                {"name": "SCOPE_ATTEMPT_ID", "value": "attempt_1"},
                {"name": "SCOPE_ATTEMPT_DEADLINE_UNIX", "value": "86400"}
            ])
        );
    }
}

#[test]
fn retry_is_unblocked_only_after_ecs_reports_the_task_stopped() {
    let running = Task::builder().last_status("RUNNING").build();
    assert!(!task_has_stopped(&[running], &[], "task-1").unwrap());

    let stopped = Task::builder().last_status("STOPPED").build();
    assert!(task_has_stopped(&[stopped], &[], "task-1").unwrap());

    let missing = Failure::builder().arn("task-1").reason("MISSING").build();
    assert!(task_has_stopped(&[], &[missing], "task-1").unwrap());

    let denied = Failure::builder()
        .arn("task-1")
        .reason("ACCESS_DENIED")
        .build();
    assert!(task_has_stopped(&[], &[denied], "task-1").is_err());
}
