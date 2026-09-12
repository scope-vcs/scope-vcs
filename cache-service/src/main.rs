use scope_cache_service::{AppState, Settings, router};
use scope_service_runtime::{init_tracing, port_from_env, serve};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing("scope_cache_service=info,tower_http=info");

    let port = port_from_env(8080);
    let state = AppState::from_settings(Settings::from_env()?).await?;
    state.start_reconciler();
    serve(port, router(state), "cache service").await
}
