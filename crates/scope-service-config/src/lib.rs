use url::{Host, Url};

/// A service base URL: HTTPS, or HTTP to an actual loopback host. Credentials,
/// query parameters and fragments never belong in deployment endpoints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceEndpoint(Url);

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct EndpointError(&'static str);

impl ServiceEndpoint {
    pub fn parse(value: &str) -> Result<Self, EndpointError> {
        let url = Url::parse(value).map_err(|_| EndpointError("endpoint must be a valid URL"))?;
        let loopback = match url.host() {
            Some(Host::Domain("localhost")) => true,
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            _ => false,
        };
        if !(url.scheme() == "https" || (url.scheme() == "http" && loopback)) {
            return Err(EndpointError(
                "endpoint must use HTTPS outside loopback development",
            ));
        }
        if url.host().is_none()
            || url.cannot_be_a_base()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(EndpointError(
                "endpoint must have a host and no credentials, query or fragment",
            ));
        }
        Ok(Self(url))
    }

    pub fn parse_origin(value: &str) -> Result<Self, EndpointError> {
        let endpoint = Self::parse(value)?;
        if endpoint.0.path() != "/" {
            return Err(EndpointError("endpoint origin must not contain a path"));
        }
        Ok(endpoint)
    }

    /// Canonical base URL for callers that append a route beginning with '/'.
    pub fn as_str(&self) -> &str {
        self.0.as_str().trim_end_matches('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_the_destination_instead_of_a_hostname_prefix() {
        for invalid in [
            "http://127.0.0.1.example.net",
            "http://127.0.0.1@remote.example",
            "https://user:secret@example.com",
            "https://",
            "http://example.com",
            "https://example.com?q=1",
            "https://example.com#fragment",
        ] {
            assert!(ServiceEndpoint::parse(invalid).is_err(), "{invalid}");
        }
        for valid in [
            "https://example.com",
            "http://127.0.0.1:8080",
            "http://127.0.0.2",
            "http://[::1]:8080",
            "http://localhost:8080",
        ] {
            assert!(ServiceEndpoint::parse(valid).is_ok(), "{valid}");
        }
    }

    #[test]
    fn canonicalizes_endpoints_and_keeps_base_path_semantics_explicit() {
        assert_eq!(
            ServiceEndpoint::parse("https://EXAMPLE.com:443/cache/")
                .unwrap()
                .as_str(),
            "https://example.com/cache"
        );
        assert!(ServiceEndpoint::parse_origin("https://example.com/cache").is_err());
        assert_eq!(
            ServiceEndpoint::parse_origin("https://EXAMPLE.com:443/")
                .unwrap()
                .as_str(),
            "https://example.com"
        );
    }
}
