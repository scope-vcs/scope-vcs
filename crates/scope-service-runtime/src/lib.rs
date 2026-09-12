//! Delivery infrastructure shared by the HTTP services and background workers.
mod bootstrap;
pub mod http;

pub use bootstrap::{init_tracing, port_from_env, serve, shutdown_signal};

pub fn unix_now() -> Result<u64, std::time::SystemTimeError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
}
