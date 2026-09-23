use super::{ProductAnalyticsError, ProductAnalyticsSink};
use crate::ProductEvent;
use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::{
    sync::{Mutex, mpsc, oneshot},
    task::JoinHandle,
    time::{sleep, timeout},
};

const QUEUE_CAPACITY: usize = 128;
const MAX_EVENT_BYTES: usize = 16 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(100), Duration::from_millis(250)];

enum Message {
    Event(String),
    Shutdown(oneshot::Sender<()>),
}

pub(super) struct PostHogSink {
    sender: mpsc::Sender<Message>,
    worker: Mutex<Option<JoinHandle<()>>>,
    token: String,
    closed: AtomicBool,
}

impl PostHogSink {
    pub(super) fn new(token: String, host: Option<String>) -> anyhow::Result<Self> {
        let mut endpoint =
            reqwest::Url::parse(host.as_deref().unwrap_or("https://us.i.posthog.com"))?;
        endpoint.set_path("/e/");
        endpoint.set_query(None);
        endpoint.set_fragment(None);
        if !matches!(endpoint.scheme(), "https" | "http") {
            anyhow::bail!("invalid PostHog host scheme");
        }
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let worker = tokio::spawn(run(receiver, client, endpoint));
        Ok(Self {
            sender,
            worker: Mutex::new(Some(worker)),
            token,
            closed: AtomicBool::new(false),
        })
    }
}

impl ProductAnalyticsSink for PostHogSink {
    fn capture(&self, event: ProductEvent) -> Result<(), ProductAnalyticsError> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(ProductAnalyticsError("analytics delivery is closed"));
        }
        let mut properties = event.properties;
        properties.insert("token".into(), Value::String(self.token.clone()));
        properties.insert("$process_person_profile".into(), Value::Bool(false));
        properties.insert("$geoip_disable".into(), Value::Bool(true));
        let body = json!({
            "api_key": self.token,
            "event": event.name,
            "distinct_id": event.distinct_id,
            "properties": properties,
            "uuid": uuid::Uuid::new_v4().to_string(),
            "timestamp": OffsetDateTime::now_utc().format(&Rfc3339)
                .map_err(|_| ProductAnalyticsError("invalid event timestamp"))?,
        });
        let payload = serde_json::to_string(&body)
            .map_err(|_| ProductAnalyticsError("invalid event payload"))?;
        if payload.len() > MAX_EVENT_BYTES {
            return Err(ProductAnalyticsError(
                "event exceeds analytics payload limit",
            ));
        }
        self.sender
            .try_send(Message::Event(payload))
            .map_err(|_| ProductAnalyticsError("analytics queue is full or closed"))
    }

    fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.closed.store(true, Ordering::Relaxed);
            let mut worker = self.worker.lock().await;
            let Some(handle) = worker.take() else {
                return;
            };
            let (finished, received) = oneshot::channel();
            let drain = async {
                self.sender.send(Message::Shutdown(finished)).await.ok();
                received.await.ok();
            };
            if timeout(SHUTDOWN_TIMEOUT, drain).await.is_err() {
                handle.abort();
            }
            let _ = handle.await;
        })
    }
}

async fn run(
    mut receiver: mpsc::Receiver<Message>,
    client: reqwest::Client,
    endpoint: reqwest::Url,
) {
    while let Some(message) = receiver.recv().await {
        match message {
            Message::Event(payload) => deliver(&client, &endpoint, payload).await,
            Message::Shutdown(finished) => {
                let _ = finished.send(());
                return;
            }
        }
    }
}

async fn deliver(client: &reqwest::Client, endpoint: &reqwest::Url, payload: String) {
    for retry_delay in RETRY_DELAYS.into_iter().map(Some).chain([None]) {
        let response = client
            .post(endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload.clone())
            .send()
            .await;
        let retryable = match response {
            Ok(response) if response.status().is_success() => return,
            Ok(response) => {
                let status = response.status();
                tracing::warn!(
                    status = status.as_u16(),
                    "PostHog product analytics delivery failed"
                );
                status.is_server_error() || status.as_u16() == 429
            }
            Err(_) => {
                tracing::warn!("PostHog product analytics delivery failed");
                true
            }
        };
        if !retryable {
            return;
        }
        let Some(delay) = retry_delay else {
            return;
        };
        sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, http::StatusCode, routing::post};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn bounded_queue_and_payload_drop_excess_events() {
        let (sender, _receiver) = mpsc::channel(1);
        let sink = PostHogSink {
            sender,
            worker: Mutex::new(None),
            token: "phc_test".into(),
            closed: AtomicBool::new(false),
        };
        assert!(
            sink.capture(ProductEvent::account_created("scope_usr_one"))
                .is_ok()
        );
        assert!(
            sink.capture(ProductEvent::account_created("scope_usr_one"))
                .is_err()
        );
        assert!(
            sink.capture(
                ProductEvent::account_created("scope_usr_one")
                    .with_request_id(&"x".repeat(MAX_EVENT_BYTES))
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn delivery_retries_transient_rejection_and_preserves_private_payload() {
        let calls = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
        let app = Router::new().route(
            "/e/",
            post({
                let calls = calls.clone();
                let bodies = bodies.clone();
                move |Json(body): Json<Value>| {
                    let calls = calls.clone();
                    let bodies = bodies.clone();
                    async move {
                        bodies.lock().await.push(body);
                        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                            StatusCode::SERVICE_UNAVAILABLE
                        } else {
                            StatusCode::OK
                        }
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let sink = PostHogSink::new("phc_test".into(), Some(host)).unwrap();

        sink.capture(ProductEvent::account_created("scope_usr_one"))
            .unwrap();
        sink.shutdown().await;

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let bodies = bodies.lock().await;
        assert_eq!(bodies[0]["event"], "account:user_create");
        assert_eq!(bodies[0]["distinct_id"], "scope_usr_one");
        assert_eq!(bodies[0]["properties"]["$geoip_disable"], true);
        assert_eq!(bodies[0]["properties"]["$process_person_profile"], false);
        assert_eq!(bodies[0]["properties"]["token"], "phc_test");
        assert_eq!(bodies[0]["uuid"], bodies[1]["uuid"]);
        assert_eq!(bodies[0]["timestamp"], bodies[1]["timestamp"]);
        assert!(bodies[0]["uuid"].as_str().is_some());
        assert!(bodies[0]["timestamp"].as_str().is_some());
        assert!(
            sink.capture(ProductEvent::account_created("scope_usr_one"))
                .is_err()
        );
        server.abort();
    }

    #[tokio::test]
    async fn permanent_rejection_is_not_retried() {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route(
            "/e/",
            post({
                let calls = calls.clone();
                move || {
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        StatusCode::BAD_REQUEST
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let sink = PostHogSink::new("phc_test".into(), Some(host)).unwrap();
        sink.capture(ProductEvent::account_created("scope_usr_one"))
            .unwrap();
        sink.shutdown().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn shutdown_has_a_deadline_when_delivery_stalls() {
        let app = Router::new().route(
            "/e/",
            post(|| async {
                sleep(Duration::from_secs(5)).await;
                StatusCode::OK
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let sink = PostHogSink::new("phc_test".into(), Some(host)).unwrap();
        sink.capture(ProductEvent::account_created("scope_usr_one"))
            .unwrap();
        let started = std::time::Instant::now();
        sink.shutdown().await;
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(
            sink.capture(ProductEvent::account_created("scope_usr_one"))
                .is_err()
        );
        server.abort();
    }
}
