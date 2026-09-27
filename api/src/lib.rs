pub mod app;
pub mod maintenance_http;
pub mod state;

pub(crate) mod auth;
pub(crate) mod cache_grants;
pub(crate) mod clerk_users;
pub(crate) mod config;
#[cfg(any(test, feature = "local-dev", feature = "smoke-seed"))]
#[path = "dev/seed.rs"]
pub(crate) mod demo_seed;
#[cfg(feature = "local-dev")]
pub mod dev;
#[cfg(any(feature = "local-dev", feature = "smoke-seed"))]
mod env_guard;
pub(crate) mod error;
pub(crate) mod git;
pub(crate) mod git_segment_recovery;
pub(crate) mod http;
pub(crate) mod invite_mailer;
pub(crate) mod media_grants;
pub(crate) mod object_store_config;
pub(crate) mod operation_analytics;
pub(crate) mod persistence;
pub(crate) mod persistence_ids;
pub(crate) mod push_intents;
pub(crate) mod repo_access;
pub(crate) mod repo_events;
mod repository_backfill;
mod request_auto_merge_runtime;
pub(crate) mod retention;
pub(crate) mod run_attempt_effects;
pub(crate) mod run_recovery;
pub(crate) mod runtime_budgets;
#[cfg(feature = "smoke-seed")]
pub mod smoke_seed;
mod storage_runtime;
pub(crate) mod telemetry;
pub(crate) mod use_cases;
mod workflow_catalog_backfill;

#[cfg(test)]
mod workflow_tests;

pub use app::router;
pub use request_auto_merge_runtime::RequestAutoMergeRuntime;
pub use state::AppState;
pub use workflow_catalog_backfill::{
    backfill_repository_workflow_catalogs_for_maintenance,
    validate_repository_workflow_catalogs_for_maintenance,
};

#[cfg(feature = "type-export")]
pub fn export_api_contract(output_path: &std::path::Path, schema_output_path: &std::path::Path) {
    http::type_exports::export_api_contract(output_path, schema_output_path);
}
