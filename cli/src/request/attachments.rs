mod journal;
use super::text::terminal_text;
use crate::api::{
    RequestTarget, finish_request_attachment, get_request_attachment,
    get_request_attachment_limits, prepare_request_attachment, upload_request_attachment_part,
};
use anyhow::{Context, bail};
pub(super) use journal::{begin_mutation, complete_mutation, complete_uploads};
use journal::{fingerprint, rotate_upload_operation, unix_now, upload_operations};
use scope_api_contract::attachments::{
    FinishRequestAttachmentRequest, PrepareRequestAttachmentRequest, RequestAttachmentKind,
    RequestAttachmentPartReceiptResponse, RequestAttachmentResponse, RequestAttachmentState,
    RequestAttachmentTargetInput,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const MAX_PART_BYTES: usize = 8 * 1024 * 1024;
const HASH_BUFFER_BYTES: usize = 1024 * 1024;
const WAIT_TIMEOUT: Duration = Duration::from_secs(120);
const WAIT_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub(super) struct UploadedAttachments {
    pub(super) attachments: Vec<RequestAttachmentResponse>,
    pub(super) references: Vec<String>,
    pub(super) receipt_keys: Vec<String>,
}

#[derive(Clone)]
struct AttachmentFile {
    path: PathBuf,
    filename: String,
    declared_media_type: String,
    kind: RequestAttachmentKind,
    size_bytes: u64,
    sha256: String,
    receipt_key: String,
}

pub(super) fn upload(
    client: &reqwest::blocking::Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
    attachment_target: RequestAttachmentTargetInput,
    paths: Vec<PathBuf>,
) -> anyhow::Result<UploadedAttachments> {
    if paths.is_empty() {
        return Ok(UploadedAttachments {
            attachments: Vec::new(),
            references: Vec::new(),
            receipt_keys: Vec::new(),
        });
    }

    let limits = get_request_attachment_limits(client, api_url, session_token, target)?;
    if paths.len() > limits.max_attachments_per_content {
        bail!(
            "at most {} attachments may be added to one request description or message",
            limits.max_attachments_per_content
        );
    }
    let target_json =
        serde_json::to_string(&attachment_target).context("serialize request attachment target")?;
    let mut files = Vec::with_capacity(paths.len());
    let mut seen = BTreeSet::new();
    for path in paths {
        let file = inspect_file(path, api_url, target, &target_json)?;
        let max_bytes = match file.kind {
            RequestAttachmentKind::Photo => limits.max_photo_bytes,
            RequestAttachmentKind::Video => limits.max_video_bytes,
        };
        let accepted_media_types = match file.kind {
            RequestAttachmentKind::Photo => &limits.accepted_photo_media_types,
            RequestAttachmentKind::Video => &limits.accepted_video_media_types,
        };
        if !accepted_media_types.contains(&file.declared_media_type) {
            bail!(
                "attachment {} has media type {}, which this Scope server does not accept",
                file.path.display(),
                file.declared_media_type
            );
        }
        if file.size_bytes > max_bytes {
            bail!(
                "{} is too large ({} bytes; maximum is {} bytes)",
                file.path.display(),
                file.size_bytes,
                max_bytes
            );
        }
        if seen.insert(file.receipt_key.clone()) {
            files.push(file);
        }
    }

    let operations = upload_operations(
        &files
            .iter()
            .map(|file| file.receipt_key.as_str())
            .collect::<Vec<_>>(),
    )?;

    let mut attachments = Vec::with_capacity(files.len());
    let mut references = Vec::with_capacity(files.len());
    for (file, operation_id) in files.iter().zip(operations) {
        let mut prepare_request = PrepareRequestAttachmentRequest {
            operation_id,
            target: attachment_target.clone(),
            filename: file.filename.clone(),
            declared_media_type: file.declared_media_type.clone(),
            size_bytes: file.size_bytes,
            sha256: file.sha256.clone(),
        };
        let prepared = match prepare_request_attachment(
            client,
            api_url,
            session_token,
            target,
            &prepare_request,
        ) {
            Ok(prepared) => prepared,
            Err(error)
                if crate::error::response(&error).code
                    == scope_api_contract::ErrorCode::AttachmentUploadExpired =>
            {
                prepare_request.operation_id =
                    rotate_upload_operation(&file.receipt_key, &prepare_request.operation_id)?;
                prepare_request_attachment(
                    client,
                    api_url,
                    session_token,
                    target,
                    &prepare_request,
                )?
            }
            Err(error) => return Err(error),
        };
        let attachment = if prepared.attachment.state == RequestAttachmentState::Prepared {
            transfer_and_finish(
                client,
                api_url,
                session_token,
                target,
                file,
                &prepare_request,
                prepared,
            )?
        } else {
            eprintln!(
                "Reusing {} · {}",
                terminal_text(&file.filename),
                prepared.attachment.id
            );
            prepared.attachment
        };
        references.push(markdown_reference(&attachment));
        attachments.push(attachment);
    }
    Ok(UploadedAttachments {
        attachments,
        references,
        receipt_keys: files.into_iter().map(|file| file.receipt_key).collect(),
    })
}

fn transfer_and_finish(
    client: &reqwest::blocking::Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
    file: &AttachmentFile,
    prepare_request: &PrepareRequestAttachmentRequest,
    mut prepared: scope_api_contract::attachments::PrepareRequestAttachmentResponse,
) -> anyhow::Result<RequestAttachmentResponse> {
    let attachment_id = prepared.attachment.id.clone();
    let mut refreshes = 0_u8;
    'transfer: loop {
        if prepared.attachment.state != RequestAttachmentState::Prepared {
            return Ok(prepared.attachment);
        }
        let transfer = prepared.transfer.clone();
        let preferred_part_bytes = usize::try_from(transfer.preferred_part_bytes)
            .unwrap_or(usize::MAX)
            .min(MAX_PART_BYTES);
        if preferred_part_bytes == 0 {
            bail!("attachment media service returned a zero-byte preferred part size");
        }
        let acknowledged = validated_acknowledged_parts(
            &transfer.acknowledged_parts,
            file.size_bytes,
            preferred_part_bytes,
        )?;
        let mut receipts = Vec::new();
        let mut input = File::open(&file.path)
            .with_context(|| format!("open attachment {}", file.path.display()))?;
        let mut complete_hasher = Sha256::new();
        let mut uploaded_bytes = 0_u64;
        let mut part_number = 1_u32;
        loop {
            let bytes = read_part(&mut input, preferred_part_bytes)
                .with_context(|| format!("read attachment {}", file.path.display()))?;
            if bytes.is_empty() {
                break;
            }
            complete_hasher.update(&bytes);
            let part_sha256 = hex::encode(Sha256::digest(&bytes));
            let size_bytes = bytes.len() as u64;
            let receipt = if let Some(receipt) = acknowledged.get(&part_number) {
                if receipt.size_bytes != size_bytes || receipt.sha256 != part_sha256 {
                    bail!(
                        "saved upload part {} no longer matches {}; the local file changed during retry",
                        part_number,
                        file.path.display()
                    );
                }
                receipt.clone()
            } else {
                if unix_now()?.saturating_add(15) >= transfer.expires_at_unix {
                    prepared = refresh_preparation(
                        client,
                        api_url,
                        session_token,
                        target,
                        prepare_request,
                        &attachment_id,
                        &mut refreshes,
                    )?;
                    continue 'transfer;
                }
                match upload_request_attachment_part(
                    client,
                    &transfer.media_base_url,
                    &transfer.grant,
                    &transfer.upload_id,
                    part_number,
                    bytes,
                ) {
                    Ok(receipt) => {
                        if receipt.part_number != part_number
                            || receipt.size_bytes != size_bytes
                            || receipt.sha256 != part_sha256
                        {
                            bail!("attachment media service returned an invalid part receipt");
                        }
                        receipt
                    }
                    Err(error) if is_expired_transfer_grant(&error) => {
                        prepared = refresh_preparation(
                            client,
                            api_url,
                            session_token,
                            target,
                            prepare_request,
                            &attachment_id,
                            &mut refreshes,
                        )?;
                        continue 'transfer;
                    }
                    Err(error) => return Err(error),
                }
            };
            uploaded_bytes = uploaded_bytes
                .checked_add(size_bytes)
                .context("attachment upload byte count overflowed")?;
            print_progress(&file.filename, uploaded_bytes, file.size_bytes);
            receipts.push(receipt);
            part_number = part_number
                .checked_add(1)
                .context("attachment has too many upload parts")?;
        }
        if uploaded_bytes != file.size_bytes
            || hex::encode(complete_hasher.finalize()) != file.sha256
        {
            bail!(
                "{} changed while it was being uploaded; rerun the command to start a new upload",
                file.path.display()
            );
        }
        return finish_request_attachment(
            client,
            api_url,
            session_token,
            target,
            &attachment_id,
            &FinishRequestAttachmentRequest {
                upload_id: transfer.upload_id,
                parts: receipts,
            },
        );
    }
}

fn refresh_preparation(
    client: &reqwest::blocking::Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
    request: &PrepareRequestAttachmentRequest,
    attachment_id: &str,
    refreshes: &mut u8,
) -> anyhow::Result<scope_api_contract::attachments::PrepareRequestAttachmentResponse> {
    *refreshes = refreshes.saturating_add(1);
    if *refreshes > 32 {
        bail!("attachment transfer grant expired too many times; retry the command");
    }
    let refreshed = prepare_request_attachment(client, api_url, session_token, target, request)?;
    if refreshed.attachment.id != attachment_id {
        bail!("attachment API changed the attachment for an existing operation ID");
    }
    eprintln!("Renewed media transfer grant and reconciled uploaded parts.");
    Ok(refreshed)
}

fn is_expired_transfer_grant(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<crate::error::CliError>()
        .is_some_and(|error| error.response().code == scope_api_contract::ErrorCode::Unauthorized)
}

fn validated_acknowledged_parts(
    receipts: &[RequestAttachmentPartReceiptResponse],
    file_size: u64,
    part_size: usize,
) -> anyhow::Result<BTreeMap<u32, RequestAttachmentPartReceiptResponse>> {
    let expected_parts = file_size.div_ceil(part_size as u64);
    let mut result = BTreeMap::new();
    for receipt in receipts {
        if receipt.part_number == 0 || u64::from(receipt.part_number) > expected_parts {
            bail!("attachment API returned an out-of-range acknowledged part");
        }
        if result
            .insert(receipt.part_number, receipt.clone())
            .is_some()
        {
            bail!("attachment API returned duplicate acknowledged parts");
        }
    }
    Ok(result)
}

fn read_part(input: &mut File, part_bytes: usize) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; part_bytes];
    let mut filled = 0;
    while filled < bytes.len() {
        let read = input.read(&mut bytes[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    bytes.truncate(filled);
    Ok(bytes)
}

fn inspect_file(
    path: PathBuf,
    api_url: &str,
    target: RequestTarget<'_>,
    target_json: &str,
) -> anyhow::Result<AttachmentFile> {
    let metadata =
        fs::metadata(&path).with_context(|| format!("inspect attachment {}", path.display()))?;
    if !metadata.is_file() {
        bail!("attachment {} must be a regular file", path.display());
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .with_context(|| format!("attachment {} has no UTF-8 file name", path.display()))?
        .to_string();
    let declared_media_type = media_type_for_path(&path)?.to_string();
    let kind = if declared_media_type.starts_with("image/") {
        RequestAttachmentKind::Photo
    } else {
        RequestAttachmentKind::Video
    };
    let (size_bytes, sha256) = hash_file(&path)?;
    if size_bytes != metadata.len() {
        bail!(
            "{} changed while it was being inspected; rerun the command",
            path.display()
        );
    }
    let receipt_key = fingerprint(&[
        api_url,
        target.owner,
        target.repo,
        target.request_id,
        target_json,
        &filename,
        &size_bytes.to_string(),
        &sha256,
    ]);
    Ok(AttachmentFile {
        path,
        filename,
        declared_media_type,
        kind,
        size_bytes,
        sha256,
        receipt_key,
    })
}

fn hash_file(path: &Path) -> anyhow::Result<(u64, String)> {
    let mut input =
        File::open(path).with_context(|| format!("open attachment {}", path.display()))?;
    let mut buffer = vec![0; HASH_BUFFER_BYTES];
    let mut hasher = Sha256::new();
    let mut size_bytes = 0_u64;
    loop {
        let read = input
            .read(&mut buffer)
            .with_context(|| format!("read attachment {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size_bytes = size_bytes
            .checked_add(read as u64)
            .context("attachment size overflowed")?;
    }
    Ok((size_bytes, hex::encode(hasher.finalize())))
}

fn media_type_for_path(path: &Path) -> anyhow::Result<&'static str> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => Ok("image/png"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "webp" => Ok("image/webp"),
        "gif" => Ok("image/gif"),
        "heic" => Ok("image/heic"),
        "heif" => Ok("image/heif"),
        "mp4" => Ok("video/mp4"),
        "mov" => Ok("video/quicktime"),
        "webm" => Ok("video/webm"),
        _ => bail!(
            "attachment {} must be PNG, JPEG, WebP, GIF, HEIC, HEIF, MP4, MOV, or WebM",
            path.display()
        ),
    }
}

fn markdown_reference(attachment: &RequestAttachmentResponse) -> String {
    let label = attachment
        .filename
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    let destination = format!("/request-attachments/{}", attachment.id);
    match attachment.kind {
        RequestAttachmentKind::Photo => format!("![{label}]({destination})"),
        RequestAttachmentKind::Video => format!("[{label}]({destination})"),
    }
}

pub(super) fn wait_for_processing(
    client: &reqwest::blocking::Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
    attachments: Vec<RequestAttachmentResponse>,
    recovery: serde_json::Value,
) -> anyhow::Result<Vec<RequestAttachmentResponse>> {
    wait_for_processing_with_policy(
        client,
        api_url,
        session_token,
        target,
        attachments,
        recovery,
        WAIT_TIMEOUT,
        WAIT_POLL_INTERVAL,
    )
}

#[allow(clippy::too_many_arguments)]
fn wait_for_processing_with_policy(
    client: &reqwest::blocking::Client,
    api_url: &str,
    session_token: &str,
    target: RequestTarget<'_>,
    mut attachments: Vec<RequestAttachmentResponse>,
    mut recovery: serde_json::Value,
    timeout: Duration,
    poll_interval: Duration,
) -> anyhow::Result<Vec<RequestAttachmentResponse>> {
    let deadline = Instant::now() + timeout;
    let mut last_states = attachments
        .iter()
        .map(|attachment| attachment.state)
        .collect::<Vec<_>>();
    while attachments.iter().any(is_processing) && Instant::now() < deadline {
        thread::sleep(poll_interval);
        for index in 0..attachments.len() {
            if is_processing(&attachments[index]) {
                let attachment_id = attachments[index].id.clone();
                let loaded = match get_request_attachment(
                    client,
                    api_url,
                    session_token,
                    target,
                    &attachment_id,
                ) {
                    Ok(loaded) => loaded,
                    Err(error) => {
                        let mut response = crate::error::response(&error);
                        response.message = format!(
                            "request content was saved, but attachment status could not be loaded: {}; rerun the same command to resume waiting",
                            response.message
                        );
                        return Err(saved_processing_error(
                            response,
                            &mut recovery,
                            &attachments,
                        )?);
                    }
                };
                attachments[index] = loaded;
                if attachments[index].state != last_states[index] {
                    eprintln!(
                        "Processing {} · {:?}",
                        terminal_text(&attachments[index].filename),
                        attachments[index].state
                    );
                    last_states[index] = attachments[index].state;
                }
            }
        }
    }
    if attachments.iter().any(is_processing) {
        return Err(saved_processing_error(
            scope_api_contract::ErrorResponse::new(
                scope_api_contract::ErrorCode::ServiceUnavailable,
                "request content was saved, but media is still processing after the bounded wait; rerun the same command to resume waiting",
            )
            .retryable(),
            &mut recovery,
            &attachments,
        )?);
    }
    Ok(attachments)
}

fn saved_processing_error(
    response: scope_api_contract::ErrorResponse,
    recovery: &mut serde_json::Value,
    attachments: &[RequestAttachmentResponse],
) -> anyhow::Result<anyhow::Error> {
    let recovery = recovery
        .as_object_mut()
        .context("build saved request attachment recovery receipt")?;
    recovery.insert(
        "attachments".to_string(),
        serde_json::to_value(attachments).context("serialize attachment recovery receipt")?,
    );
    Ok(
        crate::error::CliError::with_recovery(
            response,
            serde_json::Value::Object(recovery.clone()),
        )
        .into(),
    )
}

fn is_processing(attachment: &RequestAttachmentResponse) -> bool {
    matches!(
        attachment.state,
        RequestAttachmentState::Uploaded | RequestAttachmentState::Processing
    )
}

fn print_progress(filename: &str, uploaded_bytes: u64, total_bytes: u64) {
    let percent = uploaded_bytes
        .saturating_mul(100)
        .checked_div(total_bytes)
        .unwrap_or(100);
    eprintln!(
        "Uploading {} · {uploaded_bytes}/{total_bytes} bytes · {percent}%",
        terminal_text(filename)
    );
}

#[cfg(test)]
mod tests {
    use super::{
        fingerprint, markdown_reference, media_type_for_path, wait_for_processing_with_policy,
    };
    use crate::api::RequestTarget;
    use scope_api_contract::attachments::RequestAttachmentResponse;
    use scope_api_contract::attachments::{RequestAttachmentKind, RequestAttachmentState};
    use serde_json::json;
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn operation_fingerprint_preserves_field_boundaries() {
        assert_ne!(fingerprint(&["ab", "c"]), fingerprint(&["a", "bc"]));
    }

    #[test]
    fn supported_extensions_map_to_declared_media_types() {
        assert_eq!(
            media_type_for_path(Path::new("shot.JPEG")).unwrap(),
            "image/jpeg"
        );
        assert_eq!(
            media_type_for_path(Path::new("clip.mov")).unwrap(),
            "video/quicktime"
        );
        assert!(media_type_for_path(Path::new("notes.txt")).is_err());
    }

    #[test]
    fn markdown_uses_stable_attachment_routes() {
        let base = json!({
            "id":"att_one", "request_id":"req_one", "uploader_user_id":"usr_one",
            "filename":"before [wide].png", "declared_media_type":"image/png",
            "detected_media_type":null, "kind":"Photo", "size_bytes":12, "sha256":"a".repeat(64),
            "state":"Ready", "original_download_available":true,
            "failure":null, "image":null, "video":null, "derivatives":[],
            "created_at_unix":1, "updated_at_unix":1
        });
        let photo: RequestAttachmentResponse = serde_json::from_value(base).unwrap();
        assert_eq!(
            markdown_reference(&photo),
            "![before \\[wide\\].png](/request-attachments/att_one)"
        );

        let mut video = photo;
        video.kind = RequestAttachmentKind::Video;
        video.state = RequestAttachmentState::Processing;
        assert_eq!(
            markdown_reference(&video),
            "[before \\[wide\\].png](/request-attachments/att_one)"
        );
    }

    #[test]
    fn processing_timeout_reports_the_saved_request_and_attachment() {
        let attachment: RequestAttachmentResponse = serde_json::from_value(json!({
            "id":"att_one", "request_id":"req_one", "uploader_user_id":"usr_one",
            "filename":"walkthrough.mp4", "declared_media_type":"video/mp4",
            "detected_media_type":"video/mp4", "kind":"Video", "size_bytes":12,
            "sha256":"a".repeat(64), "state":"Processing",
            "original_download_available":true, "failure":null, "image":null,
            "video":{"width":100,"height":80,"duration_millis":1000}, "derivatives":[],
            "created_at_unix":1, "updated_at_unix":1
        }))
        .unwrap();
        let error = wait_for_processing_with_policy(
            &reqwest::blocking::Client::new(),
            "http://127.0.0.1:9",
            "token",
            RequestTarget {
                owner: "owner",
                repo: "repo",
                request_id: "req_one",
            },
            vec![attachment],
            json!({"operation":"request.edit", "saved":true, "request_id":"req_one"}),
            Duration::ZERO,
            Duration::ZERO,
        )
        .unwrap_err();

        assert_eq!(crate::error::exit_code(&error), 6);
        let envelope = serde_json::to_value(crate::error::json_response(&error)).unwrap();
        assert_eq!(envelope["retryable"], true);
        assert_eq!(envelope["recovery"]["saved"], true);
        assert_eq!(envelope["recovery"]["request_id"], "req_one");
        assert_eq!(envelope["recovery"]["attachments"][0]["id"], "att_one");
    }
}
