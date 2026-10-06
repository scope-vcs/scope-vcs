mod bootstrap;
pub mod http;
pub mod outbound_http;
pub mod readiness;
mod telemetry;

pub use bootstrap::{port_from_env, serve, shutdown_signal};
pub use telemetry::{Telemetry, init_telemetry, request_tracing};

pub fn unix_now() -> Result<u64, std::time::SystemTimeError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
}
