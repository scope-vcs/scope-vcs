use super::*;

impl RequestBasePublicationHeaders {
    pub(crate) fn parse(headers: &HeaderMap) -> Result<Option<Self>, ApiError> {
        let names = [
            "scope-request-base",
            "scope-request-base-version",
            "scope-request-parent-base-version",
            "scope-request-parent-head",
        ];
        let mut values = Vec::with_capacity(names.len());
        for name in names {
            let mut entries = headers.get_all(name).iter();
            let value = entries.next();
            if entries.next().is_some() {
                return Err(ApiError::bad_request(
                    "duplicate request base publication header",
                ));
            }
            values.push(
                value.map(|value| value.to_str()).transpose().map_err(|_| {
                    ApiError::bad_request("invalid request base publication header")
                })?,
            );
        }
        if values.iter().all(Option::is_none) {
            return Ok(None);
        }
        let selection = required(values[0])?;
        let expected_base_version = version(required(values[1])?)?;
        let (base, expected_parent_base_version) =
            if let Some(main) = selection.strip_prefix("main:") {
                if values[2].is_some() || values[3].is_some() {
                    return Err(ApiError::bad_request(
                        "main selection cannot carry parent headers",
                    ));
                }
                (
                    RequestBase::Main {
                        review_base_oid: oid(main)?,
                    },
                    None,
                )
            } else {
                let (request_id, revision_id) = selection
                    .strip_prefix("request:")
                    .and_then(|value| value.split_once('@'))
                    .filter(|(request, revision)| valid_id(request) && valid_id(revision))
                    .ok_or_else(|| ApiError::bad_request("invalid request base selection"))?;
                (
                    RequestBase::Request {
                        request_id: request_id.to_string(),
                        revision_id: revision_id.to_string(),
                        head_oid: oid(required(values[3])?)?,
                    },
                    Some(version(required(values[2])?)?),
                )
            };
        Ok(Some(Self {
            base,
            expected_base_version,
            expected_parent_base_version,
        }))
    }
}

fn required(value: Option<&str>) -> Result<&str, ApiError> {
    value.ok_or_else(|| ApiError::bad_request("request base publication headers are incomplete"))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn version(value: &str) -> Result<u64, ApiError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ApiError::bad_request("invalid request base version"));
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|version| *version > 0)
        .ok_or_else(|| ApiError::bad_request("invalid request base version"))
}

fn oid(value: &str) -> Result<String, ApiError> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request("invalid request base head"));
    }
    Ok(value.to_ascii_lowercase())
}
