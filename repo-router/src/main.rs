use scope_repo_router::{RouterConfig, router};
use scope_service_runtime::{init_telemetry, port_from_env, serve};

fn main() -> anyhow::Result<()> {
    let telemetry = init_telemetry("scope_repo_router=info")?;
    let result = run();
    telemetry.shutdown();
    result
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
    let port = port_from_env(8080);
    let app = router(RouterConfig::from_env()?)?;
    serve(port, app, "Git router").await
}
