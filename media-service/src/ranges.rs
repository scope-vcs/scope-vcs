use crate::error::ServiceError;
use axum::http::{HeaderMap, header::RANGE};
use std::ops::RangeInclusive;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RequestedRange {
    Full,
    Partial(RangeInclusive<u64>),
}

pub(crate) fn requested_range(
    headers: &HeaderMap,
    size_bytes: u64,
) -> Result<RequestedRange, ServiceError> {
    let mut values = headers.get_all(RANGE).iter();
    let Some(value) = values.next() else {
        return Ok(RequestedRange::Full);
    };
    if values.next().is_some() {
        return Err(ServiceError::range_not_satisfiable(size_bytes));
    }
    let value = value
        .to_str()
        .map_err(|_| ServiceError::range_not_satisfiable(size_bytes))?;
    let value = value
        .strip_prefix("bytes=")
        .filter(|value| !value.is_empty() && !value.contains(','))
        .ok_or_else(|| ServiceError::range_not_satisfiable(size_bytes))?;
    if size_bytes == 0 {
        return Err(ServiceError::range_not_satisfiable(size_bytes));
    }
    let (start, end) = value
        .split_once('-')
        .ok_or_else(|| ServiceError::range_not_satisfiable(size_bytes))?;
    let range = match (start.is_empty(), end.is_empty()) {
        (false, false) => {
            let start = parse_u64(start, size_bytes)?;
            let end = parse_u64(end, size_bytes)?;
            if start > end || start >= size_bytes {
                return Err(ServiceError::range_not_satisfiable(size_bytes));
            }
            start..=end.min(size_bytes - 1)
        }
        (false, true) => {
            let start = parse_u64(start, size_bytes)?;
            if start >= size_bytes {
                return Err(ServiceError::range_not_satisfiable(size_bytes));
            }
            start..=size_bytes - 1
        }
        (true, false) => {
            let suffix = parse_u64(end, size_bytes)?;
            if suffix == 0 {
                return Err(ServiceError::range_not_satisfiable(size_bytes));
            }
            size_bytes.saturating_sub(suffix)..=size_bytes - 1
        }
        (true, true) => return Err(ServiceError::range_not_satisfiable(size_bytes)),
    };
    Ok(RequestedRange::Partial(range))
}

fn parse_u64(value: &str, size_bytes: u64) -> Result<u64, ServiceError> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ServiceError::range_not_satisfiable(size_bytes));
    }
    value
        .parse()
        .map_err(|_| ServiceError::range_not_satisfiable(size_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn parse(value: Option<&str>, size: u64) -> Result<RequestedRange, ServiceError> {
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert(RANGE, HeaderValue::from_str(value).unwrap());
        }
        requested_range(&headers, size)
    }

    #[test]
    fn parses_closed_open_and_suffix_ranges() {
        assert_eq!(parse(None, 100).unwrap(), RequestedRange::Full);
        assert_eq!(
            parse(Some("bytes=10-19"), 100).unwrap(),
            RequestedRange::Partial(10..=19)
        );
        assert_eq!(
            parse(Some("bytes=90-"), 100).unwrap(),
            RequestedRange::Partial(90..=99)
        );
        assert_eq!(
            parse(Some("bytes=-10"), 100).unwrap(),
            RequestedRange::Partial(90..=99)
        );
        assert_eq!(
            parse(Some("bytes=90-200"), 100).unwrap(),
            RequestedRange::Partial(90..=99)
        );
    }

    #[test]
    fn rejects_multiple_malformed_and_unsatisfiable_ranges() {
        for value in [
            "bytes=100-",
            "bytes=20-10",
            "bytes=-0",
            "bytes=1-2,4-5",
            "items=1-2",
            "bytes=wat",
        ] {
            assert_eq!(
                parse(Some(value), 100).unwrap_err().status(),
                axum::http::StatusCode::RANGE_NOT_SATISFIABLE
            );
        }
        assert!(parse(Some("bytes=0-0"), 0).is_err());

        let mut headers = HeaderMap::new();
        headers.append(RANGE, HeaderValue::from_static("bytes=0-1"));
        headers.append(RANGE, HeaderValue::from_static("bytes=3-4"));
        assert!(requested_range(&headers, 100).is_err());
    }
}
