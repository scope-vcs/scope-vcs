pub mod api;
pub mod auth;
pub mod build;
pub mod clone;
pub mod context;
pub mod distribution;
pub mod error;
pub mod execution;
pub mod git_credential;
pub mod git_repo;
pub mod git_transport;
pub mod git_version;
pub mod init;
pub mod inspection;
pub mod installers;
pub mod licenses;
pub mod local_dependency_analysis;
pub mod login;
pub mod progress;
pub mod pull;
pub mod push;
pub mod repo_config;
pub mod repository_views;
pub mod request;
pub mod review;
pub mod run;
pub mod view;
pub mod visibility;

mod display;

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod test_support;
