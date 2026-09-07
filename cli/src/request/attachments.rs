use super::text::terminal_text;
use crate::api::{
    RequestTarget, finish_request_attachment, get_request_attachment,
    get_request_attachment_limits, prepare_request_attachment, upload_request_attachment_part,
};
use anyhow::{Context, bail};
use scope_api_contract::attachments::{
    FinishRequestAttachmentRequest, PrepareRequestAttachmentRequest, RequestAttachmentKind,
    RequestAttachmentPartReceiptResponse, RequestAttachmentResponse, RequestAttachmentState,
    RequestAttachmentTargetInput,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_PART_BYTES: usize = 8 * 1024 * 1024;
const HASH_BUFFER_BYTES: usize = 1024 * 1024;
const MAX_JOURNAL_BYTES: u64 = 2 * 1024 * 1024;
const MAX_UPLOAD_RECEIPTS: usize = 256;
const MAX_PENDING_MUTATIONS: usize = 64;
const WAIT_TIMEOUT: Duration = Duration::from_secs(120);
const WAIT_POLL_INTERVAL: Duration = Duration::from_secs(1);
const JOURNAL_KIND: &str = "scope.request-attachment-receipts";
const JOURNAL_VERSION: u8 = 1;

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

#[derive(Deserialize, Serialize)]
struct ReceiptJournal {
    kind: String,
    version: u8,
    uploads: Vec<UploadReceipt>,
    pending_mutations: Vec<PendingMutationReceipt>,
}

#[derive(Deserialize, Serialize)]
struct UploadReceipt {
    key: String,
    operation_id: String,
    updated_at_unix: u64,
}

#[derive(Deserialize, Serialize)]
struct PendingMutationReceipt {
    key: String,
    client_id: String,
    updated_at_unix: u64,
}

pub(super) struct PendingMutation {
    key: String,
    pub(super) client_id: String,
}

impl Default for ReceiptJournal {
    fn default() -> Self {
        Self {
            kind: JOURNAL_KIND.to_string(),
            version: JOURNAL_VERSION,
            uploads: Vec::new(),
            pending_mutations: Vec::new(),
        }
    }
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

    let journal_path = journal_path()?;
    let mut journal = load_journal(&journal_path)?;
    let now = unix_now()?;
    let operations = files
        .iter()
        .map(|file| journal.upload_operation_id(&file.receipt_key, now))
        .collect::<anyhow::Result<Vec<_>>>()?;
    save_journal(&journal_path, &mut journal)?;

    let mut attachments = Vec::with_capacity(files.len());
    let mut references = Vec::with_capacity(files.len());
    for (file, operation_id) in files.iter().zip(operations) {
        let prepare_request = PrepareRequestAttachmentRequest {
            operation_id,
            target: attachment_target.clone(),
            filename: file.filename.clone(),
            declared_media_type: file.declared_media_type.clone(),
            size_bytes: file.size_bytes,
            sha256: file.sha256.clone(),
        };
        let prepared =
            prepare_request_attachment(client, api_url, session_token, target, &prepare_request)?;
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

pub(super) fn begin_mutation(
    api_url: &str,
    scope_fields: &[&str],
    id_prefix: &str,
) -> anyhow::Result<PendingMutation> {
    let mut key_fields = Vec::with_capacity(scope_fields.len() + 1);
    key_fields.push(api_url);
    key_fields.extend_from_slice(scope_fields);
    let key = fingerprint(&key_fields);
    let path = journal_path()?;
    let mut journal = load_journal(&path)?;
    let now = unix_now()?;
    let client_id = if let Some(receipt) = journal
        .pending_mutations
        .iter_mut()
        .find(|receipt| receipt.key == key)
    {
        receipt.updated_at_unix = now;
        receipt.client_id.clone()
    } else {
        let client_id = fresh_id(id_prefix)?;
        journal.pending_mutations.insert(
            0,
            PendingMutationReceipt {
                key: key.clone(),
                client_id: client_id.clone(),
                updated_at_unix: now,
            },
        );
        client_id
    };
    save_journal(&path, &mut journal)?;
    Ok(PendingMutation { key, client_id })
}

pub(super) fn complete_uploads(receipt_keys: &[String]) -> anyhow::Result<()> {
    let path = journal_path()?;
    let mut journal = load_journal(&path)?;
    journal
        .uploads
        .retain(|receipt| !receipt_keys.contains(&receipt.key));
    save_journal(&path, &mut journal)
}

pub(super) fn complete_mutation(
    mutation: &PendingMutation,
    receipt_keys: &[String],
) -> anyhow::Result<()> {
    let path = journal_path()?;
    let mut journal = load_journal(&path)?;
    journal
        .uploads
        .retain(|receipt| !receipt_keys.contains(&receipt.key));
    journal
        .pending_mutations
        .retain(|receipt| receipt.key != mutation.key);
    save_journal(&path, &mut journal)
}

fn fingerprint(fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    hex::encode(hasher.finalize())
}

impl ReceiptJournal {
    fn upload_operation_id(&mut self, key: &str, now: u64) -> anyhow::Result<String> {
        if let Some(receipt) = self.uploads.iter_mut().find(|receipt| receipt.key == key) {
            receipt.updated_at_unix = now;
            return Ok(receipt.operation_id.clone());
        }
        let operation_id = fresh_id("cli_attachment")?;
        self.uploads.insert(
            0,
            UploadReceipt {
                key: key.to_string(),
                operation_id: operation_id.clone(),
                updated_at_unix: now,
            },
        );
        Ok(operation_id)
    }

    fn prune(&mut self) {
        self.uploads
            .sort_by_key(|receipt| std::cmp::Reverse(receipt.updated_at_unix));
        self.uploads.truncate(MAX_UPLOAD_RECEIPTS);
        self.pending_mutations
            .sort_by_key(|receipt| std::cmp::Reverse(receipt.updated_at_unix));
        self.pending_mutations.truncate(MAX_PENDING_MUTATIONS);
    }
}

fn fresh_id(prefix: &str) -> anyhow::Result<String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|error| anyhow::anyhow!("generate attachment operation ID: {error}"))?;
    Ok(format!("{prefix}_{}", hex::encode(random)))
}

fn load_journal(path: &Path) -> anyhow::Result<ReceiptJournal> {
    reject_symlink(path, "Scope attachment receipt journal")?;
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_JOURNAL_BYTES => {
            bail!("Scope attachment receipt journal is unexpectedly large")
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ReceiptJournal::default());
        }
        Err(error) => return Err(error).context("inspect Scope attachment receipt journal"),
    }
    let bytes = fs::read(path).context("read Scope attachment receipt journal")?;
    let journal: ReceiptJournal =
        serde_json::from_slice(&bytes).context("parse Scope attachment receipt journal")?;
    if journal.kind != JOURNAL_KIND || journal.version != JOURNAL_VERSION {
        bail!("Scope attachment receipt journal has an unsupported format");
    }
    Ok(journal)
}

fn save_journal(path: &Path, journal: &mut ReceiptJournal) -> anyhow::Result<()> {
    journal.prune();
    let bytes =
        serde_json::to_vec(journal).context("serialize Scope attachment receipt journal")?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        bail!("Scope attachment receipt journal exceeded its size bound");
    }
    let parent = path
        .parent()
        .context("Scope attachment receipt journal path has no parent")?;
    ensure_journal_directory(parent)?;
    reject_symlink(path, "Scope attachment receipt journal")?;
    let temp_path = parent.join(format!(
        ".request-attachments.{}.{}.tmp",
        std::process::id(),
        unix_now()?
    ));
    let result = (|| -> anyhow::Result<()> {
        write_private_file(&temp_path, &bytes)?;
        replace_journal_file(&temp_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn replace_journal_file(temp_path: &Path, path: &Path) -> anyhow::Result<()> {
    #[cfg(not(windows))]
    {
        fs::rename(temp_path, path).context("replace Scope attachment receipt journal")
    }
    #[cfg(windows)]
    {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context("replace Scope attachment receipt journal");
            }
        }
        fs::rename(temp_path, path).context("replace Scope attachment receipt journal")
    }
}

fn journal_path() -> anyhow::Result<PathBuf> {
    let base = non_empty_path(env::var_os("XDG_CONFIG_HOME"))
        .or_else(|| non_empty_path(env::var_os("HOME")).map(|path| path.join(".config")))
        .or_else(|| non_empty_path(env::var_os("USERPROFILE")).map(|path| path.join(".config")))
        .context("locate Scope attachment receipt journal; set XDG_CONFIG_HOME or HOME")?;
    Ok(base.join("scope").join("request-attachments.json"))
}

fn non_empty_path(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|path| !path.is_empty()).map(PathBuf::from)
}

fn ensure_journal_directory(path: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("Scope attachment receipt directory cannot be a symlink")
        }
        Ok(metadata) if !metadata.is_dir() => {
            bail!("Scope attachment receipt path must be a directory")
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).context("create Scope attachment receipt directory")?;
        }
        Err(error) => return Err(error).context("inspect Scope attachment receipt directory"),
    }
    secure_directory(path)
}

fn reject_symlink(path: &Path, label: &str) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => bail!("{label} cannot be a symlink"),
        Ok(metadata) if !metadata.is_file() => bail!("{label} must be a regular file"),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspect {label}")),
    }
}

fn write_private_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .context("create temporary Scope attachment receipt journal")?;
    file.write_all(bytes)
        .context("write Scope attachment receipt journal")?;
    file.sync_all()
        .context("sync Scope attachment receipt journal")
}

fn secure_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions)
            .context("secure Scope attachment receipt directory")?;
    }
    Ok(())
}

fn unix_now() -> anyhow::Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs())
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
