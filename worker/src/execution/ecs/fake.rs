//! An HTTP provider fixture exercising the production AWS client and retry paths.
use super::*;
use axum::{Json, Router, body::Bytes, extract::State, http::HeaderMap, routing::post};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Notify, Semaphore};

pub(crate) const TEST_IMAGE: &str =
    "scope/test@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[derive(Default)]
struct Requests {
    entries: Mutex<Vec<(String, Value)>>,
    changed: Notify,
    active_starts: AtomicUsize,
    peak_starts: AtomicUsize,
}

#[derive(Clone)]
struct ProviderState {
    requests: Arc<Requests>,
    starts: Arc<Semaphore>,
}

pub(crate) struct FakeEcs {
    pub(crate) client: EcsClient,
    pub(crate) starts: Arc<Semaphore>,
    requests: Arc<Requests>,
    server: tokio::task::JoinHandle<()>,
}

impl FakeEcs {
    pub(crate) async fn new() -> Self {
        Self::with_registry_credentials(None).await
    }

    pub(crate) async fn with_registry_credentials(
        registry_credentials_secret_arn: Option<&str>,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Requests::default());
        let starts = Arc::new(Semaphore::new(0));
        let app = Router::new()
            .route("/", post(handle))
            .with_state(ProviderState {
                requests: requests.clone(),
                starts: starts.clone(),
            });
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let credentials =
            aws_sdk_ecs::config::Credentials::new("test", "test", None, None, "fixture");
        let sdk_config = aws_config::SdkConfig::builder()
            .region(Region::new("us-east-1"))
            .credentials_provider(aws_sdk_ecs::config::SharedCredentialsProvider::new(
                credentials,
            ))
            .behavior_version(BehaviorVersion::latest())
            .endpoint_url(endpoint)
            .build();
        let settings = CloudExecutionSettings {
            api_url: "https://scope.test".into(),
            aws_region: "us-east-1".into(),
            ecs_cluster_arn: "arn:aws:ecs:us-east-1:123456789012:cluster/test".into(),
            ecs_subnet_ids: vec!["subnet-test".into()],
            ecs_security_group_id: "sg-test".into(),
            ecs_execution_role_arn: "arn:aws:iam::123456789012:role/test".into(),
            ecs_log_group: "/scope/test".into(),
            ecs_secret_name_key: [7; 32],
            registry_credentials_secret_arn: registry_credentials_secret_arn.map(str::to_string),
            runtime_version: "test".into(),
            max_concurrency: 4,
        };
        Self {
            client: EcsClient {
                client: EcsSdkClient::new(&sdk_config),
                secrets: SecretsManagerClient::new(&sdk_config),
                settings,
            },
            starts,
            requests,
            server,
        }
    }

    pub(crate) fn settings(&self) -> CloudExecutionSettings {
        self.client.settings.clone()
    }

    pub(crate) fn count(&self, method: &str) -> usize {
        self.requests
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(recorded_method, _)| recorded_method == method)
            .count()
    }

    pub(crate) fn peak_starts(&self) -> usize {
        self.requests.peak_starts.load(Ordering::SeqCst)
    }

    pub(crate) fn created_secrets(&self) -> Vec<String> {
        self.requests
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _)| method == "CreateSecret")
            .map(|(_, body)| body["SecretString"].as_str().unwrap().to_owned())
            .collect()
    }

    pub(crate) fn request_body(&self, method: &str) -> Value {
        let matching = self
            .requests
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(recorded_method, _)| recorded_method == method)
            .map(|(_, body)| body.clone())
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1, "expected exactly one {method} request");
        matching.into_iter().next().unwrap()
    }

    pub(crate) async fn wait_for(&self, method: &str, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let changed = self.requests.changed.notified();
                if self.count(method) >= count {
                    return;
                }
                changed.await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "expected {count} {method} calls, got {}",
                self.count(method)
            )
        });
    }
}

impl Drop for FakeEcs {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn handle(
    State(state): State<ProviderState>,
    headers: HeaderMap,
    body: Bytes,
) -> Json<Value> {
    let body: Value = serde_json::from_slice(&body).unwrap();
    let method = headers["x-amz-target"]
        .to_str()
        .unwrap()
        .rsplit('.')
        .next()
        .unwrap();
    state
        .requests
        .entries
        .lock()
        .unwrap()
        .push((method.into(), body.clone()));
    state.requests.changed.notify_waiters();
    Json(match method {
        // Empty discovery invokes the actual five-minute ambiguity reconciliation.
        "ListTasks" => json!({"taskArns": []}),
        "CreateSecret" => {
            json!({"ARN": "arn:aws:secretsmanager:us-east-1:123456789012:secret:test"})
        }
        "RegisterTaskDefinition" => {
            json!({"taskDefinition": {"taskDefinitionArn": "arn:aws:ecs:us-east-1:123456789012:task-definition/test:1"}})
        }
        "RunTask" => {
            let active = state.requests.active_starts.fetch_add(1, Ordering::SeqCst) + 1;
            state
                .requests
                .peak_starts
                .fetch_max(active, Ordering::SeqCst);
            state.starts.acquire().await.unwrap().forget();
            state.requests.active_starts.fetch_sub(1, Ordering::SeqCst);
            json!({"tasks": [{"taskArn": format!("task-{}", body["startedBy"].as_str().unwrap())}]})
        }
        "StopTask" => json!({}),
        "DescribeTasks" => json!({"tasks": [{"lastStatus": "STOPPED"}]}),
        "ListTaskDefinitions" => json!({"taskDefinitionArns": []}),
        "DeleteSecret" => json!({}),
        method => panic!("unexpected provider method {method}"),
    })
}
