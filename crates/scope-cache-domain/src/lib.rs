//! Durable cache-plane rules, independent of HTTP, persistence, and object storage.

mod decisions;
mod error;
mod policy;
mod types;

pub use decisions::*;
pub use error::*;
pub use policy::*;
pub use types::*;

#[cfg(test)]
#[test]
fn staging_cache_experiment_executes_current_source() {
    if let Ok(expected) = std::env::var("SCOPE_CACHE_EXPERIMENT_REVISION") {
        assert_eq!("one", expected, "restored cache executed stale test code");
    }
}
