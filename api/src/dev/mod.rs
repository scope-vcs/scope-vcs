use crate::storage_runtime::{StorageRuntime, StorageSource};
mod env;

use crate::demo_seed as seed;
use crate::{
    AppState,
    auth::cli::CliAuthService,
    config::{LOCAL_API_ORIGIN, git_public_url_from_env},
    error::ApiError,
    persistence::unix_now,
    state::AppServices,
};
use axum::{
    Json,
    extract::{Path, State},
};
use scope_api_contract::CliSessionTokenResponse;
use scope_postgres::db::MetadataStore;
use scope_product_analytics::ProductAnalytics;

pub use env::is_local_dev_env;

pub fn local_maintenance_database_url() -> anyhow::Result<String> {
    Ok(env::validate_local_dev_environment()?.database.into_url())
}

pub async fn app_state_from_env() -> anyhow::Result<AppState> {
    let settings = env::validate_local_dev_environment()?;
    let git_public_url = git_public_url_from_env(Some(LOCAL_API_ORIGIN))?;
    let storage = StorageRuntime::from_env(StorageSource::Filesystem).await?;
    storage.git_segment_store.cleanup_temporary().await?;
    let catalog = seed::catalog(
        storage.object_store.as_ref(),
        storage.git_segment_store.as_ref(),
        settings.seed_user.clone(),
    )
    .await
    .map_err(|error| {
        anyhow::anyhow!(
            "building local dev catalog: {}",
            error.into_operator_diagnostic()
        )
    })?;
    let metadata = MetadataStore::connect_local_dev(settings.database).await?;
    metadata
        .admin()
        .replace_catalog_for_seed(catalog)
        .await
        .map_err(|error| anyhow::anyhow!("seeding local dev database: {}", error.message))?;
    seed::seed_request_discussion_gallery(&metadata)
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "seeding local dev request discussions: {}",
                error.into_operator_diagnostic()
            )
        })?;
    seed::seed_run_gallery(
        &metadata,
        &settings.seed_user.handle,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| anyhow::anyhow!("reading the local dev clock: {error}"))?
            .as_secs(),
    )
    .await
    .map_err(|error| {
        anyhow::anyhow!(
            "seeding local dev runs: {}",
            error.into_operator_diagnostic()
        )
    })?;
    AppState::connect(AppServices {
        storage,
        metadata,
        run_caches: crate::use_cases::run_caches::RunCacheStorage::unreachable(),
        media_grants: crate::media_grants::MediaGrantIssuer::local()?,
        product_analytics: ProductAnalytics::disabled(),
        git_public_url,
    })
}

pub(crate) async fn create_bench_cli_session(
    State(state): State<AppState>,
) -> Result<Json<CliSessionTokenResponse>, ApiError> {
    if !env::is_local_dev_env() {
        return Err(ApiError::not_found(
            "local dev benchmark auth is unavailable",
        ));
    }

    let settings = env::validate_local_dev_environment().map_err(|error| {
        ApiError::internal_message(format!("validating local dev benchmark auth: {error}"))
    })?;
    let user = crate::seed_user::seed_user_account(settings.seed_user);
    mint_cli_session(&state, &user).await
}

pub(crate) async fn create_dev_cli_session(
    State(state): State<AppState>,
    Path(handle): Path<String>,
) -> Result<Json<CliSessionTokenResponse>, ApiError> {
    if !env::is_local_dev_env() {
        return Err(ApiError::not_found("local dev auth is unavailable"));
    }

    let settings = env::validate_local_dev_environment().map_err(|error| {
        ApiError::internal_message(format!("validating local dev auth: {error}"))
    })?;
    let user = seed::actor_account(settings.seed_user, &handle)
        .ok_or_else(|| ApiError::not_found("seeded local dev actor not found"))?;
    mint_cli_session(&state, &user).await
}

async fn mint_cli_session(
    state: &AppState,
    user: &scope_domain::account::UserAccount,
) -> Result<Json<CliSessionTokenResponse>, ApiError> {
    let now_unix = unix_now()?;
    let auth = CliAuthService::new(state.metadata.auth());
    let grant = auth.create_exchange_grant(user, now_unix).await?;
    let token = auth.exchange_grant(&grant.exchange_token, now_unix).await?;

    Ok(Json(CliSessionTokenResponse {
        session_token: token.session_token,
        expires_at_unix: token.expires_at_unix,
        identity: token.identity.into(),
    }))
}
