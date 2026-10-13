use super::*;

pub(super) async fn api_request(
    app: axum::Router,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<&str>,
) -> Response {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(bearer) = bearer {
        request = request.header(AUTHORIZATION, bearer);
    }
    let body = match body {
        Some(json) => {
            request = request.header(CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    app.oneshot(request.body(body).unwrap()).await.unwrap()
}

pub(super) async fn get(app: axum::Router, uri: &str, bearer: Option<&str>) -> Response {
    api_request(app, "GET", uri, bearer, None).await
}
