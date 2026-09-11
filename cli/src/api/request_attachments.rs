use super::{RequestTarget, execute_json_request};
use crate::api::ApiSession;
use anyhow::{Context, bail};
use reqwest::{Url, blocking::Client};
use scope_api_contract::attachments::{
    FinishRequestAttachmentRequest, PrepareRequestAttachmentRequest,
    PrepareRequestAttachmentResponse, RequestAttachmentLimitsResponse,
    RequestAttachmentPartReceiptResponse, RequestAttachmentResponse,
};
use std::time::Duration;

pub fn get_request_attachment_limits(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
) -> anyhow::Result<RequestAttachmentLimitsResponse> {
    super::requests::execute_request(
        api.request(
            reqwest::Method::GET,
            scope_api_contract::routes::repo_request_attachment_limits(
                target.owner,
                target.repo,
                target.request_id,
            ),
        ),
        target,
        "load request attachment limits",
    )
}

pub fn prepare_request_attachment(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    request: &PrepareRequestAttachmentRequest,
) -> anyhow::Result<PrepareRequestAttachmentResponse> {
    super::requests::execute_request(
        api.request(
            reqwest::Method::POST,
            scope_api_contract::routes::repo_request_attachment_prepare(
                target.owner,
                target.repo,
                target.request_id,
            ),
        )
        .json(request),
        target,
        "prepare request attachment",
    )
}

pub fn finish_request_attachment(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    attachment_id: &str,
    request: &FinishRequestAttachmentRequest,
) -> anyhow::Result<RequestAttachmentResponse> {
    super::requests::execute_request(
        api.request(
            reqwest::Method::POST,
            scope_api_contract::routes::repo_request_attachment_finish(
                target.owner,
                target.repo,
                target.request_id,
                attachment_id,
            ),
        )
        .json(request),
        target,
        "finish request attachment",
    )
}

pub fn get_request_attachment(
    api: ApiSession<'_>,
    target: RequestTarget<'_>,
    attachment_id: &str,
) -> anyhow::Result<RequestAttachmentResponse> {
    super::requests::execute_request(
        api.request(
            reqwest::Method::GET,
            scope_api_contract::routes::repo_request_attachment(
                target.owner,
                target.repo,
                target.request_id,
                attachment_id,
            ),
        ),
        target,
        "load request attachment",
    )
}

pub fn upload_request_attachment_part(
    client: &Client,
    media_base_url: &str,
    grant: &str,
    upload_id: &str,
    part_number: u32,
    bytes: Vec<u8>,
) -> anyhow::Result<RequestAttachmentPartReceiptResponse> {
    let url = upload_part_url(media_base_url, upload_id, part_number)?;
    execute_json_request(
        client
            .put(url)
            .bearer_auth(grant)
            .timeout(Duration::from_secs(120))
            .body(bytes),
        "upload request attachment part",
    )
}

fn upload_part_url(media_base_url: &str, upload_id: &str, part_number: u32) -> anyhow::Result<Url> {
    let mut url = Url::parse(media_base_url).context("parse attachment media service URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("attachment media service returned an invalid upload URL");
    }
    let part_number = part_number.to_string();
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| anyhow::anyhow!("attachment media service returned an invalid upload URL"))?;
    segments.pop_if_empty();
    segments.extend(["v1", "uploads", upload_id, "parts", &part_number]);
    drop(segments);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::upload_part_url;

    #[test]
    fn media_part_url_encodes_opaque_upload_ids() {
        assert_eq!(
            upload_part_url("https://media.example/base/", "upload /one", 2)
                .unwrap()
                .as_str(),
            "https://media.example/base/v1/uploads/upload%20%2Fone/parts/2"
        );
    }
}
