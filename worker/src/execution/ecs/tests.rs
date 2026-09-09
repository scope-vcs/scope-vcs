use super::*;

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
async fn start_sends_the_complete_task_definition_and_launch_contract() {
    for registry_credentials in [None, Some(REGISTRY_SECRET_ARN)] {
        let provider = fake::FakeEcs::with_registry_credentials(registry_credentials).await;
        provider.starts.add_permits(1);

        let external_run_id = provider
            .client
            .start(IMAGE, "attempt_1", "scope_bootstrap_test", 86_400)
            .await
            .unwrap();

        assert_eq!(external_run_id, "task-attempt_1");
        let mut expected_container = serde_json::json!({
            "name": CONTAINER_NAME,
            "image": IMAGE,
            "essential": true,
            "entryPoint": [RUNTIME_ENTRYPOINT],
            "secrets": [{
                "name": BOOTSTRAP_SECRET_ENV,
                "valueFrom": "arn:aws:secretsmanager:us-east-1:123456789012:secret:test"
            }],
            "logConfiguration": {
                "logDriver": "awslogs",
                "options": {
                    "awslogs-group": "/scope/test",
                    "awslogs-region": "us-east-1",
                    "awslogs-stream-prefix": "runner"
                }
            }
        });
        if let Some(arn) = registry_credentials {
            expected_container["repositoryCredentials"] =
                serde_json::json!({"credentialsParameter": arn});
        }
        assert_eq!(
            provider.request_body("RegisterTaskDefinition"),
            serde_json::json!({
                "family": "scope-runner-attempt_1",
                "networkMode": "awsvpc",
                "requiresCompatibilities": ["FARGATE"],
                "cpu": TASK_CPU,
                "memory": TASK_MEMORY,
                "executionRoleArn": "arn:aws:iam::123456789012:role/test",
                "runtimePlatform": {
                    "cpuArchitecture": "X86_64",
                    "operatingSystemFamily": "LINUX"
                },
                "containerDefinitions": [expected_container],
                "tags": [
                    {"key": "Project", "value": "scope-vcs"},
                    {"key": "Component", "value": "cloud-runner"},
                    {"key": "ImageDigest", "value": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}
                ]
            })
        );
        assert_eq!(
            provider.request_body("RunTask"),
            serde_json::json!({
                "cluster": "arn:aws:ecs:us-east-1:123456789012:cluster/test",
                "taskDefinition": "arn:aws:ecs:us-east-1:123456789012:task-definition/test:1",
                "launchType": "FARGATE",
                "platformVersion": "LATEST",
                "count": 1,
                "clientToken": "attempt_1",
                "startedBy": "attempt_1",
                "enableECSManagedTags": true,
                "networkConfiguration": {
                    "awsvpcConfiguration": {
                        "assignPublicIp": "ENABLED",
                        "subnets": ["subnet-test"],
                        "securityGroups": ["sg-test"]
                    }
                },
                "overrides": {
                    "containerOverrides": [{
                        "name": CONTAINER_NAME,
                        "environment": [
                            {"name": "SCOPE_API_URL", "value": "https://scope.test"},
                            {"name": "SCOPE_ATTEMPT_ID", "value": "attempt_1"},
                            {"name": "SCOPE_ATTEMPT_DEADLINE_UNIX", "value": "86400"}
                        ]
                    }]
                },
                "tags": [
                    {"key": "Project", "value": "scope-vcs"},
                    {"key": "Component", "value": "cloud-runner"},
                    {"key": "AttemptId", "value": "attempt_1"}
                ]
            })
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

#[test]
fn ambiguous_start_polling_uses_bounded_exponential_backoff() {
    let mut delay = CONSISTENCY_INITIAL_DELAY;
    let mut observed = Vec::new();
    for _ in 0..6 {
        observed.push(delay.as_secs());
        delay = next_consistency_delay(delay);
    }
    assert_eq!(observed, [2, 4, 8, 16, 30, 30]);
}
