pub mod cleanup;
pub mod codec;
pub mod config;
pub mod health;
pub mod jobs;
mod lease;
pub mod process;
mod runtime;
pub mod scratch;
pub mod storage;

pub fn unix_now() -> anyhow::Result<u64> {
    Ok(scope_service_runtime::unix_now()?)
}

pub use scope_service_runtime::shutdown_signal;
