use anyhow::Context;
use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use scope_cli::{
    distribution::DistributionManifest,
    installers::{posix_install_script, windows_install_script},
};
use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

#[derive(Clone)]
struct AppState {
    artifact_dir: Arc<PathBuf>,
    manifest: &'static DistributionManifest,
    public_url: Arc<str>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);
    let state = AppState {
        artifact_dir: Arc::new(
            env::var("SCOPE_CLI_ARTIFACT_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("./dist")),
        ),
        manifest: DistributionManifest::bundled(),
        public_url: validated_public_url(
            &env::var("SCOPE_CLI_PUBLIC_URL")
                .context("set SCOPE_CLI_PUBLIC_URL to the public CLI service origin")?,
        )?
        .into(),
    };
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/install.sh", get(install))
        .route("/install.ps1", get(install_windows))
        .route("/downloads/{artifact}", get(download))
        .with_state(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;

    axum::serve(listener, app)
        .await
        .context("serve CLI service")
}

async fn healthz() -> impl IntoResponse {
    (
        [("content-type", "application/json")],
        r#"{"status":"ok","service":"cli"}"#,
    )
}

async fn readyz(State(state): State<AppState>) -> Response {
    let missing = missing_downloads(&state);
    if missing.is_empty() {
        (
            StatusCode::OK,
            [("content-type", "application/json")],
            r#"{"status":"ok","service":"cli"}"#,
        )
            .into_response()
    } else {
        let body = format!(
            r#"{{"status":"unavailable","service":"cli","missing":{}}}"#,
            serde_json::to_string(&missing).unwrap_or_else(|_| "[]".to_string())
        );
        (
            StatusCode::SERVICE_UNAVAILABLE,
            [("content-type", "application/json")],
            body,
        )
            .into_response()
    }
}

async fn install(State(state): State<AppState>) -> impl IntoResponse {
    (
        [("content-type", "text/x-shellscript; charset=utf-8")],
        posix_install_script(&state.public_url, state.manifest),
    )
}

async fn install_windows(State(state): State<AppState>) -> impl IntoResponse {
    (
        [("content-type", "text/plain; charset=utf-8")],
        windows_install_script(&state.public_url, state.manifest),
    )
}

async fn download(State(state): State<AppState>, Path(artifact): Path<String>) -> Response {
    let Some(file_name) = state.manifest.downloadable_file(&artifact) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    match tokio::fs::read(state.artifact_dir.join(file_name)).await {
        Ok(bytes) => {
            let mut response = Body::from(bytes).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
            response
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn validated_public_url(value: &str) -> anyhow::Result<String> {
    let url = reqwest::Url::parse(value).context("parse SCOPE_CLI_PUBLIC_URL")?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "SCOPE_CLI_PUBLIC_URL must be an HTTP(S) origin without credentials, a path, query or fragment"
    );
    Ok(url.origin().ascii_serialization())
}

fn missing_downloads(state: &AppState) -> Vec<String> {
    state
        .manifest
        .required_downloads()
        .filter(|file_name| !state.artifact_dir.join(file_name).is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_origin_rejects_credentials_paths_and_script_syntax() {
        for value in [
            "file:///tmp",
            "https://user@example.test",
            "https://example.test/path",
            "https://example.test?query",
            "https://example.test#fragment",
            "https://$(printf injected)",
            "https://example.test/\"; injected",
            "https://example.test/`injected`",
        ] {
            assert!(validated_public_url(value).is_err(), "{value}");
        }
        assert_eq!(
            validated_public_url("https://example.test/").unwrap(),
            "https://example.test"
        );
        assert_eq!(
            validated_public_url("http://127.0.0.1:8080").unwrap(),
            "http://127.0.0.1:8080"
        );
    }
}
