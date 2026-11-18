use super::*;
use axum::http::header;

#[tokio::test]
async fn a_download_is_recorded_once_and_redirects_to_its_github_release_asset() {
    let mut state = AppState::test_state();
    let (analytics, recording) = scope_product_analytics::ProductAnalytics::recording();
    state.product_analytics = analytics;
    let app = router(state);
    let artifact = "scope-x86_64-unknown-linux-gnu.tar.gz";
    let path = format!("/cli/downloads/cli-0123456789ab/{artifact}?installer=sh");

    let mut answers = Vec::new();
    for (method, path) in [
        ("GET", path.as_str()),
        ("HEAD", path.as_str()),
        (
            "GET",
            "/cli/downloads/cli-0123456789ab/scope-unknown.tar.gz",
        ),
        (
            "GET",
            "/cli/downloads/main/scope-x86_64-unknown-linux-gnu.tar.gz",
        ),
    ] {
        let response = api_request(app.clone(), method, path, None, None).await;
        answers.push((
            response.status(),
            response
                .headers()
                .get(header::LOCATION)
                .map(|location| location.to_str().unwrap().to_string()),
        ));
    }

    let asset = Some(format!(
        "https://github.com/scope-vcs/scope-vcs/releases/download/cli-0123456789ab/{artifact}"
    ));
    assert_eq!(
        answers,
        [
            (StatusCode::FOUND, asset.clone()),
            (StatusCode::FOUND, asset),
            (StatusCode::NOT_FOUND, None),
            (StatusCode::NOT_FOUND, None),
        ]
    );
    assert_eq!(recording.event_names(), ["cli:artifact_download"]);
    for (property, value) in [
        ("version", "cli-0123456789ab"),
        ("artifact", artifact),
        ("target", "x86_64-unknown-linux-gnu"),
        ("os", "linux"),
        ("arch", "x64"),
        ("installer", "sh"),
        ("source", "cli_downloads"),
    ] {
        assert_eq!(
            recording.property(0, property),
            Some(serde_json::Value::String(value.to_string())),
            "{property}"
        );
    }
}
