//! Delivery contracts shared by the API and its Rust clients.
//!
//! Durable policy stays in `scope-domain`; this crate owns only serialized shapes
//! and route construction.

pub mod attachments;
mod error;
mod git_oid;
mod projections;
mod repo_config;
mod runs;
mod types;
mod wire;

mod cli_compatibility;
mod cli_output;
mod dependencies;

pub mod routes;
pub use attachments::*;
pub use cli_compatibility::*;
pub use cli_output::*;
pub use dependencies::*;
pub use error::*;
pub use git_oid::*;
pub use projections::*;
pub use repo_config::*;
pub use runs::*;
pub use types::*;
pub use wire::*;
