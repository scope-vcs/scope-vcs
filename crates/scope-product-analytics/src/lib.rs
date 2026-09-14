use scope_domain::requests::{RequestActorRole, RequestAudience};
use serde_json::{Map, Value};
use transport::ProductEventContext;

mod transport;
mod workflow;

pub use transport::ProductAnalytics;
#[cfg(any(test, feature = "test-support"))]
pub use transport::RecordingProductAnalytics;
pub use workflow::{WorkflowAttemptResult, WorkflowRunResult, WorkflowRunTrigger};

const SCOPE_USER_ID_PREFIX: &str = "scope_usr_";
const SCOPE_SYSTEM_DISTINCT_ID: &str = "scope_system";
const REPOSITORY_INCARNATION_ID_PREFIX: &str = "repoi_";

#[derive(Clone, Debug, PartialEq)]
pub struct ProductEvent {
    name: &'static str,
    distinct_id: String,
    properties: Map<String, Value>,
    source: Option<EventSource>,
}

impl ProductEvent {
    pub fn account_created(actor_user_id: &str) -> Self {
        Self::new("account:user_create", ProductActor::User(actor_user_id))
    }

    pub fn repository_initialized(actor_user_id: &str, repository_id: &str) -> Self {
        Self::repository_event(
            "repository:repository_initialize",
            actor_user_id,
            repository_id,
        )
        .with_source(EventSource::Git)
    }

    pub fn repository_pushed(actor_user_id: &str, repository_id: &str) -> Self {
        Self::repository_event("repository:push_complete", actor_user_id, repository_id)
            .with_source(EventSource::Git)
    }

    pub fn repository_invite_accepted(actor_user_id: &str, repository_id: &str) -> Self {
        let mut event =
            Self::repository_event("repository:invite_accept", actor_user_id, repository_id);
        event.insert_string("actor_role", "member");
        event
    }

    pub fn cli_session_created(actor_user_id: &str, method: CliSessionMethod) -> Self {
        let mut event = Self::new("cli:session_create", ProductActor::User(actor_user_id))
            .with_source(EventSource::Cli);
        event.insert_string("authentication_method", method.as_str());
        event
    }

    pub fn request_started(
        actor_user_id: &str,
        repository_id: &str,
        request_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
    ) -> Self {
        Self::request_event(
            "request:request_start",
            actor_user_id,
            repository_id,
            request_id,
            audience,
            actor_role,
        )
    }

    pub fn request_submitted(
        actor_user_id: &str,
        repository_id: &str,
        request_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
    ) -> Self {
        Self::request_event(
            "request:request_submit",
            actor_user_id,
            repository_id,
            request_id,
            audience,
            actor_role,
        )
    }

    pub fn request_merged(
        actor_user_id: &str,
        repository_id: &str,
        request_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
    ) -> Self {
        Self::request_event(
            "request:request_merge",
            actor_user_id,
            repository_id,
            request_id,
            audience,
            actor_role,
        )
    }

    pub fn request_revised(
        actor_user_id: &str,
        repository_id: &str,
        request_id: &str,
        audience: RequestAudience,
    ) -> Self {
        let mut event = Self::new("request:revision_create", ProductActor::User(actor_user_id));
        event.insert_repository_id(repository_id);
        event.insert_string("request_id", request_id);
        event.insert_request_audience(audience);
        event.with_source(EventSource::Git)
    }

    pub fn request_rated(
        actor_user_id: &str,
        repository_id: &str,
        request_id: &str,
        audience: RequestAudience,
        score: u8,
    ) -> Self {
        let mut event = Self::new("request:rating_create", ProductActor::User(actor_user_id));
        event.insert_repository_id(repository_id);
        event.insert_string("request_id", request_id);
        event.insert_request_audience(audience);
        event.insert_number("score", score.into());
        event
    }

    pub fn discussion_created(
        actor_user_id: &str,
        repository_id: &str,
        request_id: &str,
        discussion_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
        anchored: bool,
    ) -> Self {
        let mut event = Self::discussion_event(
            "discussion:discussion_create",
            actor_user_id,
            repository_id,
            request_id,
            discussion_id,
            audience,
            actor_role,
        );
        event.insert_bool("anchored", anchored);
        event
    }

    pub fn discussion_reply_created(
        actor_user_id: &str,
        repository_id: &str,
        request_id: &str,
        discussion_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
    ) -> Self {
        Self::discussion_event(
            "discussion:reply_create",
            actor_user_id,
            repository_id,
            request_id,
            discussion_id,
            audience,
            actor_role,
        )
    }

    pub fn discussion_resolved(
        actor_user_id: &str,
        repository_id: &str,
        request_id: &str,
        discussion_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
    ) -> Self {
        Self::discussion_event(
            "discussion:discussion_resolve",
            actor_user_id,
            repository_id,
            request_id,
            discussion_id,
            audience,
            actor_role,
        )
    }

    pub fn request_closed(
        actor_user_id: &str,
        repository_id: &str,
        request_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
        outcome: RequestCloseOutcome,
    ) -> Self {
        let mut event = Self::request_event(
            "request:request_close",
            actor_user_id,
            repository_id,
            request_id,
            audience,
            actor_role,
        );
        event.insert_string("outcome", outcome.as_str());
        event
    }

    pub fn operation_failed(
        actor: ProductActor<'_>,
        operation: ProductOperation,
        reason: OperationFailureReason,
        duration_ms: u64,
    ) -> Self {
        let mut event = Self::new("operation:failure", actor);
        event.insert_string("operation", operation.as_str());
        event.insert_string("reason", reason.as_str());
        event.insert_number("duration_ms", duration_ms.into());
        event
    }

    pub fn with_repository_id(mut self, repository_id: &str) -> Self {
        self.insert_repository_id(repository_id);
        self
    }

    pub fn with_request_id(mut self, request_id: &str) -> Self {
        self.insert_string("request_id", request_id);
        self
    }

    pub fn with_source(mut self, source: EventSource) -> Self {
        self.source = Some(source);
        self
    }

    fn repository_event(name: &'static str, actor_user_id: &str, repository_id: &str) -> Self {
        let mut event = Self::new(name, ProductActor::User(actor_user_id));
        event.insert_repository_id(repository_id);
        event
    }

    fn request_event(
        name: &'static str,
        actor_user_id: &str,
        repository_id: &str,
        request_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
    ) -> Self {
        let mut event = Self::repository_event(name, actor_user_id, repository_id);
        event.insert_string("request_id", request_id);
        event.insert_request_audience(audience);
        event.insert_string("actor_role", request_actor_role_name(actor_role));
        event
    }

    #[allow(clippy::too_many_arguments)]
    fn discussion_event(
        name: &'static str,
        actor_user_id: &str,
        repository_id: &str,
        request_id: &str,
        discussion_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
    ) -> Self {
        let mut event = Self::request_event(
            name,
            actor_user_id,
            repository_id,
            request_id,
            audience,
            actor_role,
        );
        event.insert_string("discussion_id", discussion_id);
        event
    }

    fn insert_repository_id(&mut self, repository_id: &str) {
        self.insert_string("repository_id", repository_id);
    }

    fn insert_request_audience(&mut self, audience: RequestAudience) {
        self.insert_string("request_audience", request_audience_name(audience));
    }

    fn insert_string(&mut self, name: &str, value: &str) {
        self.properties
            .insert(name.to_string(), Value::String(value.to_string()));
    }

    fn insert_bool(&mut self, name: &str, value: bool) {
        self.properties.insert(name.to_string(), Value::Bool(value));
    }

    fn insert_number(&mut self, name: &str, value: serde_json::Number) {
        self.properties
            .insert(name.to_string(), Value::Number(value));
    }

    fn apply_context(&mut self, context: &ProductEventContext) {
        self.insert_string("environment", context.environment.as_str());
        self.insert_string("source", self.source.unwrap_or(context.source).as_str());
        if let Some(release) = context.release.as_deref() {
            self.insert_string("release", release);
        }
    }

    fn has_valid_distinct_id(&self) -> bool {
        is_opaque_id(&self.distinct_id, SCOPE_USER_ID_PREFIX)
            || self.distinct_id == SCOPE_SYSTEM_DISTINCT_ID
    }

    fn has_private_repository_id(&self) -> bool {
        self.properties.get("repository_id").is_none_or(|value| {
            value
                .as_str()
                .is_some_and(|id| is_opaque_id(id, REPOSITORY_INCARNATION_ID_PREFIX))
        })
    }

    fn new(name: &'static str, actor: ProductActor<'_>) -> Self {
        Self {
            name,
            distinct_id: actor.distinct_id().to_string(),
            properties: Map::new(),
            source: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductActor<'a> {
    User(&'a str),
    System,
}

impl ProductActor<'_> {
    fn distinct_id(&self) -> &str {
        match self {
            Self::User(user_id) => user_id,
            Self::System => SCOPE_SYSTEM_DISTINCT_ID,
        }
    }

    fn actor_type(self) -> &'static str {
        match self {
            Self::User(_) => "user",
            Self::System => "system",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalyticsEnvironment {
    Production,
    Test,
}

impl AnalyticsEnvironment {
    fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Test => "test",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventSource {
    Api,
    Cli,
    Git,
    Worker,
}

impl EventSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Cli => "cli",
            Self::Git => "git",
            Self::Worker => "worker",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliSessionMethod {
    Browser,
    Device,
    ExchangeGrant,
}

impl CliSessionMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Device => "device",
            Self::ExchangeGrant => "exchange_grant",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestCloseOutcome {
    Closed,
    DraftDeleted,
}

impl RequestCloseOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::DraftDeleted => "draft_deleted",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductOperation {
    Login,
    Push,
    Submit,
    Merge,
}

impl ProductOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Push => "push",
            Self::Submit => "submit",
            Self::Merge => "merge",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationFailureReason {
    AuthenticationRejected,
    PermissionDenied,
    InvalidInput,
    Conflict,
    RateLimited,
    Unavailable,
    Internal,
}

impl OperationFailureReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::AuthenticationRejected => "authentication_rejected",
            Self::PermissionDenied => "permission_denied",
            Self::InvalidInput => "invalid_input",
            Self::Conflict => "conflict",
            Self::RateLimited => "rate_limited",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal",
        }
    }
}

fn is_opaque_id(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    })
}

fn request_audience_name(audience: RequestAudience) -> &'static str {
    match audience {
        RequestAudience::Public => "public",
        RequestAudience::Private => "private",
    }
}

fn request_actor_role_name(role: RequestActorRole) -> &'static str {
    match role {
        RequestActorRole::Public => "public",
        RequestActorRole::Member => "member",
        RequestActorRole::Owner => "owner",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlated_request_event_contains_only_approved_context() {
        let (analytics, recording) = ProductAnalytics::recording_with_context(
            EventSource::Api,
            AnalyticsEnvironment::Test,
            Some("release_sha"),
        );
        analytics.capture(ProductEvent::request_started(
            "scope_usr_test",
            "repoi_private",
            "req_private",
            RequestAudience::Private,
            RequestActorRole::Owner,
        ));

        let event = &recording.events()[0];
        assert_eq!(event.name, "request:request_start");
        assert_eq!(event.distinct_id, "scope_usr_test");
        assert_eq!(
            event.properties,
            Map::from_iter([
                ("actor_role".to_string(), Value::String("owner".to_string())),
                ("environment".to_string(), Value::String("test".to_string())),
                (
                    "release".to_string(),
                    Value::String("release_sha".to_string())
                ),
                (
                    "repository_id".to_string(),
                    Value::String("repoi_private".to_string())
                ),
                (
                    "request_audience".to_string(),
                    Value::String("private".to_string())
                ),
                (
                    "request_id".to_string(),
                    Value::String("req_private".to_string())
                ),
                ("source".to_string(), Value::String("api".to_string())),
            ]),
        );
    }

    #[test]
    fn source_override_and_system_actor_are_explicit() {
        let (analytics, recording) = ProductAnalytics::recording();
        analytics.capture(ProductEvent::workflow_attempt_started(
            ProductActor::System,
            "repoi_test",
            "run_test",
            "attempt_test",
            2,
            WorkflowRunTrigger::PushMain,
        ));

        assert_eq!(
            recording.property(0, "source"),
            Some(Value::String("worker".into()))
        );
        assert_eq!(
            recording.property(0, "actor_type"),
            Some(Value::String("system".into()))
        );
    }

    #[test]
    fn non_scope_distinct_ids_are_rejected_before_the_sink() {
        let (analytics, recording) = ProductAnalytics::recording();
        analytics.capture(ProductEvent::repository_initialized(
            "clerk_user_secret",
            "repoi_test",
        ));

        assert!(recording.events().is_empty());
    }

    #[test]
    fn user_facing_repository_ids_are_rejected_before_the_sink() {
        let (analytics, recording) = ProductAnalytics::recording();
        for repository_id in [
            "owner/private-repo-name",
            "repoi_owner/private-repo",
            "repoi_",
        ] {
            analytics.capture(ProductEvent::repository_initialized(
                "scope_usr_test",
                repository_id,
            ));
        }

        assert!(recording.events().is_empty());
    }
}
