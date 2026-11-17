use crate::after_commit::AfterCommit;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use scope_product_analytics::{CliArtifactDownload, CliInstaller, ProductEvent};
use serde::Deserialize;
use std::sync::OnceLock;

const RELEASE_DOWNLOADS: &str = "https://github.com/scope-vcs/scope-vcs/releases/download";
const TARGETS_JSON: &str = include_str!("../../../cli/distribution/targets.json");

#[derive(Deserialize)]
struct DistributionTargets {
    targets: Vec<DistributionTarget>,
}

#[derive(Deserialize)]
struct DistributionTarget {
    triple: String,
    artifact: String,
    os: String,
    arch: String,
}

#[derive(Deserialize)]
pub(crate) struct DownloadQuery {
    installer: Option<String>,
}

#[expect(clippy::expect_used)]
fn distribution_targets() -> &'static [DistributionTarget] {
    static TARGETS: OnceLock<Vec<DistributionTarget>> = OnceLock::new();
    TARGETS.get_or_init(|| {
        serde_json::from_str::<DistributionTargets>(TARGETS_JSON)
            .expect("the bundled CLI distribution targets are valid")
            .targets
    })
}

fn is_release_tag(release: &str) -> bool {
    release.strip_prefix("cli-").is_some_and(|version| {
        (1..=64).contains(&version.len())
            && version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    })
}

pub(crate) async fn download(
    method: Method,
    State(state): State<AppState>,
    Path((release, artifact)): Path<(String, String)>,
    Query(query): Query<DownloadQuery>,
) -> Response {
    let Some(target) = distribution_targets()
        .iter()
        .find(|target| target.artifact == artifact)
        .filter(|_| is_release_tag(&release))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if method == Method::GET {
        state.after_commit([AfterCommit::Analytics(
            ProductEvent::cli_artifact_downloaded(CliArtifactDownload {
                version: &release,
                artifact: &target.artifact,
                target_triple: &target.triple,
                os: &target.os,
                arch: &target.arch,
                installer: CliInstaller::from_query(query.installer.as_deref()),
            }),
        )]);
    }
    (
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("{RELEASE_DOWNLOADS}/{release}/{}", target.artifact),
        )],
    )
        .into_response()
}
