use anyhow::Context;
use api::{AppState, router};
use scope_service_runtime::{init_tracing, shutdown_signal};
use std::net::{Ipv6Addr, SocketAddr};

fn main() -> anyhow::Result<()> {
    scope_git_process::install_pid1_reaper_if_needed()?;
    run()
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
    init_tracing("api=info,scope_postgres=info,tower_http=info");

    let port = scope_service_runtime::port_from_env(8080);
    let addr = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
    let state = app_state_from_env().await?;

    serve(addr, state).await
}

async fn app_state_from_env() -> anyhow::Result<AppState> {
    #[cfg(feature = "local-dev")]
    {
        if api::dev::is_local_dev_env() {
            return api::dev::app_state_from_env().await;
        }
    }

    #[cfg(not(feature = "local-dev"))]
    {
        if std::env::var("SCOPE_ENV").ok().as_deref() == Some("local") {
            anyhow::bail!("SCOPE_ENV=local requires running the API with --features local-dev");
        }
    }

    AppState::from_env().await
}

async fn serve(addr: SocketAddr, state: AppState) -> anyhow::Result<()> {
    let shutdown_state = state.clone();
    let app = router(state);
    tracing::info!(%addr, "starting api");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding server on {addr}"))?;
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving api");
    shutdown_state.shutdown_product_analytics().await;

    result
}
