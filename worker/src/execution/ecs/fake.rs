//! An HTTP provider fixture exercising the production AWS client and retry paths.
use super::*;
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
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
    stops: Arc<Semaphore>,
    reply: Arc<Mutex<Option<(StatusCode, Value, bool)>>>,
}

pub(crate) struct FakeEcs {
    pub(crate) client: EcsClient,
    pub(crate) starts: Arc<Semaphore>,
    pub(crate) stops: Arc<Semaphore>,
    reply: Arc<Mutex<Option<(StatusCode, Value, bool)>>>,
    requests: Arc<Requests>,
    server: tokio::task::JoinHandle<()>,
}

impl FakeEcs {
    pub(crate) async fn new() -> Self {
        Self::with_timeout(Duration::from_secs(150)).await
    }

    pub(crate) async fn with_timeout(timeout: Duration) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Requests::default());
        let starts = Arc::new(Semaphore::new(0));
        let stops = Arc::new(Semaphore::new(0));
        let reply = Arc::new(Mutex::new(None));
        let app = Router::new()
            .route("/2015-03-31/functions/{function}/invocations", post(handle))
            .with_state(ProviderState {
                requests: requests.clone(),
                starts: starts.clone(),
                stops: stops.clone(),
                reply: reply.clone(),
            });
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let credentials =
            aws_sdk_lambda::config::Credentials::new("test", "test", None, None, "fixture");
        let sdk_config = aws_config::SdkConfig::builder()
            .region(Region::new("us-east-1"))
            .credentials_provider(aws_sdk_lambda::config::SharedCredentialsProvider::new(
                credentials,
            ))
            .behavior_version(BehaviorVersion::latest())
            .retry_config(RetryConfig::standard().with_max_attempts(1))
            .timeout_config(TimeoutConfig::builder().operation_timeout(timeout).build())
            .endpoint_url(endpoint)
            .build();
        let settings = CloudExecutionSettings {
            aws_region: "us-east-1".into(),
            dispatch_broker_function_arn:
                "arn:aws:lambda:us-east-1:123456789012:function:scope-dispatch".into(),
            runtime_version: "test".into(),
            max_concurrency: 4,
        };
        Self {
            client: EcsClient {
                client: LambdaClient::new(&sdk_config),
                settings,
            },
            starts,
            stops,
            reply,
            requests,
            server,
        }
    }

    pub(crate) fn reply(&self, status: StatusCode, body: Value, function_error: bool) {
        *self.reply.lock().unwrap() = Some((status, body, function_error));
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

    pub(crate) fn bootstrap_tokens(&self) -> Vec<String> {
        self.requests
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _)| method == "start")
            .map(|(_, body)| body["bootstrap_token"].as_str().unwrap().to_owned())
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

async fn handle(State(state): State<ProviderState>, headers: HeaderMap, body: Bytes) -> Response {
    assert_eq!(headers["x-amz-invocation-type"], "RequestResponse");
    let body: Value = serde_json::from_slice(&body).unwrap();
    let method = body["action"].as_str().unwrap();
    state
        .requests
        .entries
        .lock()
        .unwrap()
        .push((method.into(), body.clone()));
    state.requests.changed.notify_waiters();
    if let Some((status, reply, function_error)) = state.reply.lock().unwrap().clone() {
        let mut response = (status, Json(reply)).into_response();
        if function_error {
            response
                .headers_mut()
                .insert("x-amz-function-error", "Unhandled".parse().unwrap());
        }
        return response;
    }
    Json(match method {
        "start" => {
            let active = state.requests.active_starts.fetch_add(1, Ordering::SeqCst) + 1;
            state.requests.peak_starts.fetch_max(active, Ordering::SeqCst);
            state.starts.acquire().await.unwrap().forget();
            state.requests.active_starts.fetch_sub(1, Ordering::SeqCst);
            json!({"status": "started", "task_arn": format!("task-{}", body["attempt_id"].as_str().unwrap())})
        }
        "stop" => {
            if body["attempt_id"] != "canceled" {
                state.stops.acquire().await.unwrap().forget();
            }
            json!({"status": "stopped"})
        }
        method => panic!("unexpected broker action {method}"),
    }).into_response()
}
