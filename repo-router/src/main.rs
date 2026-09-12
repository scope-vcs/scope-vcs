use scope_repo_router::{RouterConfig, router};
use scope_service_runtime::{init_tracing, port_from_env, serve};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing("scope_repo_router=info,tower_http=info");

    let port = port_from_env(8080);
    let app = router(RouterConfig::from_env()?)?;
    serve(port, app, "Git router").await
}
