use super::{AnalyticsEnvironment, EventSource, ProductEvent};
#[cfg(any(test, feature = "test-support"))]
use serde_json::Value;
use std::{future::Future, pin::Pin, sync::Arc};

mod delivery;
use delivery::PostHogSink;

const POSTHOG_PROJECT_TOKEN_ENV: &str = "POSTHOG_PROJECT_TOKEN";
const POSTHOG_HOST_ENV: &str = "POSTHOG_HOST";
const ANALYTICS_ENVIRONMENT_ENV: &str = "SCOPE_ANALYTICS_ENVIRONMENT";
const ANALYTICS_RELEASE_ENV: &str = "SCOPE_ANALYTICS_RELEASE";
const RAILWAY_ENVIRONMENT_NAME_ENV: &str = "RAILWAY_ENVIRONMENT_NAME";

#[derive(Clone)]
pub struct ProductAnalytics {
    sink: Arc<dyn ProductAnalyticsSink>,
    context: Option<ProductEventContext>,
}

impl ProductAnalytics {
    pub async fn from_env(source: EventSource) -> anyhow::Result<Self> {
        let Some(context) = ProductEventContext::from_env(source) else {
            return Ok(Self::disabled());
        };
        let Some(project_token) = non_empty_env(POSTHOG_PROJECT_TOKEN_ENV) else {
            tracing::warn!(
                environment = context.environment.as_str(),
                "product analytics disabled because POSTHOG_PROJECT_TOKEN is not configured"
            );
            return Ok(Self::disabled());
        };

        Ok(Self {
            sink: Arc::new(PostHogSink::new(
                project_token,
                non_empty_env(POSTHOG_HOST_ENV),
            )?),
            context: Some(context),
        })
    }

    pub fn disabled() -> Self {
        Self {
            sink: Arc::new(DisabledSink),
            context: None,
        }
    }

    pub fn capture(&self, mut event: ProductEvent) {
        if !event.has_valid_distinct_id() {
            tracing::warn!(
                event = event.name,
                "dropped product analytics event with an invalid distinct ID"
            );
            return;
        }
        if !event.has_private_repository_id() {
            tracing::warn!(
                event = event.name,
                "dropped product analytics event with a non-opaque repository ID"
            );
            return;
        }
        let Some(context) = self.context.as_ref() else {
            return;
        };
        event.apply_context(context);
        if let Err(error) = self.sink.capture(event) {
            tracing::warn!(error = %error, "product analytics capture failed");
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.context.is_some()
    }

    pub async fn shutdown(&self) {
        self.sink.shutdown().await;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn recording() -> (Self, RecordingProductAnalytics) {
        Self::recording_with_context(EventSource::Api, AnalyticsEnvironment::Test, Some("test"))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn recording_for_source(source: EventSource) -> (Self, RecordingProductAnalytics) {
        Self::recording_with_context(source, AnalyticsEnvironment::Test, Some("test"))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn recording_with_context(
        source: EventSource,
        environment: AnalyticsEnvironment,
        release: Option<&str>,
    ) -> (Self, RecordingProductAnalytics) {
        let recording = RecordingProductAnalytics::default();
        (
            Self {
                sink: Arc::new(recording.clone()),
                context: Some(ProductEventContext {
                    environment,
                    release: release.map(str::to_string),
                    source,
                }),
            },
            recording,
        )
    }
}

#[derive(Clone)]
pub(super) struct ProductEventContext {
    pub(super) environment: AnalyticsEnvironment,
    pub(super) release: Option<String>,
    pub(super) source: EventSource,
}

impl ProductEventContext {
    fn from_env(source: EventSource) -> Option<Self> {
        let configured_environment = non_empty_env(ANALYTICS_ENVIRONMENT_ENV);
        let railway_environment = non_empty_env(RAILWAY_ENVIRONMENT_NAME_ENV);
        let context = Self::resolve(
            source,
            configured_environment.as_deref(),
            railway_environment.as_deref(),
            non_empty_env(ANALYTICS_RELEASE_ENV),
        );
        if configured_environment.is_some() && context.is_none() {
            tracing::warn!(
                analytics_environment = configured_environment.as_deref(),
                railway_environment = railway_environment.as_deref(),
                "product analytics disabled because deployment environment is invalid"
            );
        }
        context
    }

    fn resolve(
        source: EventSource,
        configured_environment: Option<&str>,
        railway_environment: Option<&str>,
        release: Option<String>,
    ) -> Option<Self> {
        let environment = match configured_environment {
            Some("production") => AnalyticsEnvironment::Production,
            Some("test") => AnalyticsEnvironment::Test,
            Some(_) => return None,
            None => return None,
        };
        let mismatched_deployment = match environment {
            AnalyticsEnvironment::Production => {
                railway_environment.is_some_and(|name| name != "production")
            }
            AnalyticsEnvironment::Test => railway_environment == Some("production"),
        };
        if mismatched_deployment {
            return None;
        }
        Some(Self {
            environment,
            release,
            source,
        })
    }
}

pub(super) trait ProductAnalyticsSink: Send + Sync {
    fn capture(&self, event: ProductEvent) -> Result<(), ProductAnalyticsError>;

    fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(std::future::ready(()))
    }
}

struct DisabledSink;

impl ProductAnalyticsSink for DisabledSink {
    fn capture(&self, _event: ProductEvent) -> Result<(), ProductAnalyticsError> {
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct ProductAnalyticsError(pub(super) &'static str);

impl std::fmt::Display for ProductAnalyticsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Default)]
pub struct RecordingProductAnalytics {
    events: Arc<std::sync::Mutex<Vec<ProductEvent>>>,
}

#[cfg(any(test, feature = "test-support"))]
impl RecordingProductAnalytics {
    pub fn events(&self) -> Vec<ProductEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn event_names(&self) -> Vec<&'static str> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|event| event.name)
            .collect()
    }

    pub fn property(&self, event_index: usize, name: &str) -> Option<Value> {
        self.events
            .lock()
            .unwrap()
            .get(event_index)
            .and_then(|event| event.properties.get(name))
            .cloned()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ProductAnalyticsSink for RecordingProductAnalytics {
    fn capture(&self, event: ProductEvent) -> Result<(), ProductAnalyticsError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_environment_policy_fails_closed_across_deployments() {
        assert!(ProductEventContext::resolve(EventSource::Api, None, None, None).is_none());
        assert!(
            ProductEventContext::resolve(EventSource::Api, Some("preview"), Some("preview"), None,)
                .is_none()
        );
        assert!(
            ProductEventContext::resolve(
                EventSource::Api,
                Some("production"),
                Some("staging"),
                None,
            )
            .is_none()
        );
        assert!(
            ProductEventContext::resolve(
                EventSource::Worker,
                Some("test"),
                Some("production"),
                None,
            )
            .is_none()
        );
        let production = ProductEventContext::resolve(
            EventSource::Api,
            Some("production"),
            Some("production"),
            Some("release_sha".into()),
        )
        .unwrap();
        assert_eq!(production.environment, AnalyticsEnvironment::Production);
        assert_eq!(production.release.as_deref(), Some("release_sha"));
    }
}
