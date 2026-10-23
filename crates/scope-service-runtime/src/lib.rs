mod bootstrap;
pub mod grants;
pub mod http;
pub mod outbound_http;
pub mod proxy;
pub mod readiness;
pub mod replicas;
mod telemetry;

pub use bootstrap::{port_from_env, serve, shutdown_signal, wait_or_shutdown};
pub use telemetry::{Telemetry, init_telemetry, request_tracing};

pub fn unix_now() -> Result<u64, std::time::SystemTimeError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
}

pub fn random_hex(prefix: &str, bytes: usize) -> Result<String, getrandom::Error> {
    let mut random = vec![0_u8; bytes];
    getrandom::fill(&mut random)?;
    Ok(format!("{prefix}{}", hex::encode(random)))
}
