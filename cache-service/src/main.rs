use scope_cache_service::{AppState, Settings, router};
use scope_service_runtime::{init_telemetry, port_from_env, serve};

fn main() -> anyhow::Result<()> {
    let telemetry = init_telemetry("scope_cache_service=info")?;
    let result = run();
    telemetry.shutdown();
    result
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
    let port = port_from_env(8080);
    let state = AppState::from_settings(Settings::from_env()?).await?;
    state.start_reconciler();
    serve(port, router(state), "cache service").await
}
