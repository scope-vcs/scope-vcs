mod auth;
mod config;
mod error;
mod handlers;
mod ranges;

use config::Settings;

use auth::GrantVerifier;
use axum::{
    Router,
    extract::Request,
    http::{
        HeaderValue, Method,
        header::{
            ACCEPT_RANGES, AUTHORIZATION, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
            CONTENT_TYPE, ETAG, RANGE,
        },
    },
    middleware::{self, Next},
    response::Response,
    routing::{get, put},
};
use scope_api_contract::routes::{
    MEDIA_ATTACHMENT_DERIVATIVE, MEDIA_ATTACHMENT_ORIGINAL, MEDIA_UPLOAD_PART,
};
use scope_media_storage::MediaStorage;
use scope_postgres::db::MetadataStore;
use scope_service_runtime::{init_telemetry, port_from_env, serve};
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
pub struct AppState {
    metadata: MetadataStore,
    storage: MediaStorage,
    verifier: Arc<GrantVerifier>,
    upload_slots: Arc<Semaphore>,
    read_slots: Arc<Semaphore>,
}

impl AppState {
    pub async fn from_settings(settings: Settings) -> anyhow::Result<Self> {
        let metadata = MetadataStore::connect(settings.database_url).await?;
        let storage = settings
            .storage
            .connect(settings.max_storage_operations)
            .await?;
        Ok(Self {
            metadata,
            storage,
            verifier: Arc::new(GrantVerifier::new(&settings.grant_public_key_pem)?),
            upload_slots: Arc::new(Semaphore::new(settings.max_concurrent_uploads)),
            read_slots: Arc::new(Semaphore::new(settings.max_concurrent_reads)),
        })
    }
}

pub fn router(state: AppState, allowed_origin: &str) -> anyhow::Result<Router> {
    let router = Router::new()
        .route("/healthz", get(handlers::healthz))
        .route("/readyz", get(handlers::readyz))
        .route(MEDIA_UPLOAD_PART, put(handlers::put_upload_part))
        .route(
            MEDIA_ATTACHMENT_ORIGINAL,
            get(handlers::original).head(handlers::original),
        )
        .route(
            MEDIA_ATTACHMENT_DERIVATIVE,
            get(handlers::derivative).head(handlers::derivative),
        )
        .layer(middleware::from_fn(redacted_access_log))
        .with_state(state);
    let origin = HeaderValue::from_str(allowed_origin)?;
    Ok(router.layer(
        CorsLayer::new()
            .allow_origin(origin)
            .allow_methods([Method::GET, Method::HEAD, Method::PUT])
            .allow_headers([AUTHORIZATION, CONTENT_TYPE, RANGE])
            .max_age(Duration::from_secs(2 * 60 * 60))
            .expose_headers([
                ACCEPT_RANGES,
                CONTENT_DISPOSITION,
                CONTENT_LENGTH,
                CONTENT_RANGE,
                CONTENT_TYPE,
                ETAG,
            ]),
    ))
}

async fn redacted_access_log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let response = next.run(request).await;
    tracing::info!(%method, %path, status = response.status().as_u16(), "request completed");
    response
}

fn main() -> anyhow::Result<()> {
    let telemetry = init_telemetry("media-api", "scope_media_service=info")?;
    let result = run();
    telemetry.shutdown();
    result
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
    let port = port_from_env(8080);
    let settings = Settings::from_env()?;
    let allowed_origin = settings.allowed_origin().to_owned();
    let state = AppState::from_settings(settings).await?;
    serve(port, router(state, &allowed_origin)?, "media service").await
}
