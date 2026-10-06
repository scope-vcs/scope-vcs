use scope_media_service::{AppState, Settings, router};
use scope_service_runtime::{init_telemetry, port_from_env, serve};

fn main() -> anyhow::Result<()> {
    let telemetry = init_telemetry("scope_media_service=info")?;
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
