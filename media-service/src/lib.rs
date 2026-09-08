mod auth;
mod config;
mod error;
mod handlers;
mod ranges;

pub use config::Settings;

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
use config::MediaObjectStoreSettings;
use scope_api_contract::routes::{
    MEDIA_ATTACHMENT_DERIVATIVE, MEDIA_ATTACHMENT_ORIGINAL, MEDIA_UPLOAD_PART,
};
use scope_media_storage::MediaStorage;
use scope_object_store::{FileObjectStore, ObjectStore, S3ObjectStore};
use scope_postgres::db::MetadataStore;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tower_http::cors::CorsLayer;
use tracing::Instrument as _;

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
        let metadata = MetadataStore::connect_worker(settings.database_url).await?;
        let raw_store: Arc<dyn ObjectStore> = match settings.object_store {
            MediaObjectStoreSettings::Filesystem(settings) => {
                Arc::new(FileObjectStore::new(settings))
            }
            MediaObjectStoreSettings::S3(settings) => {
                Arc::new(tokio::task::spawn_blocking(move || S3ObjectStore::new(settings)).await??)
            }
        };
        let storage = MediaStorage::encrypted(
            raw_store,
            settings.encryption_key,
            settings.max_blocking_operations,
        )?;
        Ok(Self {
            metadata,
            storage,
            verifier: Arc::new(GrantVerifier::new(&settings.grant_public_key_pem)?),
            upload_slots: Arc::new(Semaphore::new(settings.max_concurrent_uploads)),
            read_slots: Arc::new(Semaphore::new(settings.max_concurrent_reads)),
        })
    }
}

pub fn router(state: AppState, allowed_origin: Option<&str>) -> anyhow::Result<Router> {
    let router = Router::new()
        .route("/healthz", get(handlers::healthz))
        .route("/readyz", get(handlers::readyz))
        .route(MEDIA_UPLOAD_PART, put(handlers::put_upload_part))
        .route(
            MEDIA_ATTACHMENT_ORIGINAL,
            get(handlers::get_original).head(handlers::head_original),
        )
        .route(
            MEDIA_ATTACHMENT_DERIVATIVE,
            get(handlers::get_derivative).head(handlers::head_derivative),
        )
        .layer(middleware::from_fn(redacted_access_log))
        .with_state(state);
    let Some(origin) = allowed_origin else {
        return Ok(router);
    };
    let origin = HeaderValue::from_str(origin)?;
    Ok(router.layer(
        CorsLayer::new()
            .allow_origin(origin)
            .allow_methods([Method::GET, Method::HEAD, Method::PUT])
            .allow_headers([AUTHORIZATION, CONTENT_TYPE, RANGE])
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
    let span = tracing::info_span!("media_request", %method, %path);
    let response = next.run(request).instrument(span.clone()).await;
    tracing::info!(parent: &span, status = response.status().as_u16(), "request completed");
    response
}
