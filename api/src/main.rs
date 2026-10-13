use anyhow::Context;
use api::{AppState, router};
use scope_service_runtime::{init_telemetry, shutdown_signal};
use std::net::{Ipv6Addr, SocketAddr};

#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn main() -> anyhow::Result<()> {
    scope_git_process::install_pid1_reaper_if_needed()?;
    let telemetry = init_telemetry("api", "api=info,scope_postgres=info")?;
    let result = run();
    telemetry.shutdown();
    result
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
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
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding server on {addr}"))?;
    let background_work = state.start_background_work();
    let stop_background_work = background_work.stop_signal();
    let app = router(state);
    tracing::info!(%addr, "starting api");
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = stop_background_work.send(true);
        })
        .await
        .context("serving api");
    background_work.shutdown().await;
    shutdown_state.shutdown_product_analytics().await;

    result
}
