use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, header},
    response::Response,
};
use std::collections::HashSet;

pub fn proxied_response(upstream: reqwest::Response) -> Response {
    let status = upstream.status();
    let headers = forwarded_headers(upstream.headers());
    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

pub fn forwarded_headers(headers: &HeaderMap) -> HeaderMap {
    let connection_headers = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| {
            value
                .split(',')
                .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        })
        .collect::<HashSet<_>>();
    headers
        .iter()
        .filter(|(name, _)| !is_hop_by_hop(name) && !connection_headers.contains(*name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_connection_scoped_headers_and_preserves_end_to_end_values() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "scope.example".parse().unwrap());
        headers.append(
            header::CONNECTION,
            "keep-alive, x-remove-one".parse().unwrap(),
        );
        headers.append(header::CONNECTION, "x-remove-two".parse().unwrap());
        headers.insert("keep-alive", "timeout=5".parse().unwrap());
        headers.insert("x-remove-one", "one".parse().unwrap());
        headers.insert("x-remove-two", "two".parse().unwrap());
        headers.insert("proxy-connection", "keep-alive".parse().unwrap());
        headers.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        headers.append("x-preserve", "first".parse().unwrap());
        headers.append("x-preserve", "second".parse().unwrap());

        let forwarded = forwarded_headers(&headers);

        for removed in [
            "host",
            "connection",
            "keep-alive",
            "proxy-connection",
            "x-remove-one",
            "x-remove-two",
        ] {
            assert!(!forwarded.contains_key(removed), "preserved {removed}");
        }
        assert_eq!(forwarded[header::AUTHORIZATION], "Bearer secret");
        assert_eq!(
            forwarded
                .get_all("x-preserve")
                .iter()
                .map(|value| value.to_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
    }
}
