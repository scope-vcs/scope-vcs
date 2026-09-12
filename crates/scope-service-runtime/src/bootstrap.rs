use anyhow::Context as _;
use axum::Router;
use std::net::{Ipv6Addr, SocketAddr};
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

pub fn init_tracing(default_filter: &str) {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}

pub fn port_from_env(default: u16) -> u16 {
    std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

pub async fn serve(port: u16, router: Router, service: &str) -> anyhow::Result<()> {
    let address = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("binding {service} on {address}"))?;
    tracing::info!(%address, "starting {service}");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .with_context(|| format!("serving {service}"))
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install termination handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
