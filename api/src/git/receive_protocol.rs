use crate::{error::ApiError, git::upload::pkt_line};
use axum::body::{Body, Bytes};
use futures_util::{StreamExt, stream};
use std::time::Duration;

const MAX_COMMAND_BYTES: usize = 1024 * 1024;
const FLUSH: &[u8] = b"0000";
const NO_REFS: &str = "capabilities^{}";

pub(crate) struct ReceivePackCommands {
    pub(crate) refs: Vec<String>,
    pub(crate) body: Body,
}

pub(crate) async fn read_receive_pack_commands(
    body: Body,
    deadline: Duration,
) -> Result<ReceivePackCommands, ApiError> {
    let deadline = tokio::time::Instant::now() + deadline;
    let mut chunks = body.into_data_stream();
    let mut buffered = Vec::new();
    loop {
        if let Some(refs) = command_refs(&buffered)? {
            let prefix = Bytes::from(buffered);
            let replay = stream::once(async move { Ok::<_, axum::Error>(prefix) }).chain(chunks);
            return Ok(ReceivePackCommands {
                refs,
                body: Body::from_stream(replay),
            });
        }
        if buffered.len() > MAX_COMMAND_BYTES {
            return Err(ApiError::payload_too_large(
                "git receive-pack commands are too large",
            ));
        }
        let Ok(chunk) = tokio::time::timeout_at(deadline, chunks.next()).await else {
            return Err(ApiError::infrastructure_unavailable(
                "git request upload timed out",
            ));
        };
        match chunk {
            Some(Ok(chunk)) => buffered.extend_from_slice(&chunk),
            Some(Err(error)) => {
                return Err(ApiError::bad_request(format!(
                    "invalid Git receive-pack body: {error}"
                )));
            }
            None => {
                return Err(ApiError::bad_request(
                    "git receive-pack body ended before its commands",
                ));
            }
        }
    }
}

fn command_refs(buffer: &[u8]) -> Result<Option<Vec<String>>, ApiError> {
    let invalid = || ApiError::bad_request("invalid Git receive-pack command");
    let mut refs = Vec::new();
    let mut offset = 0;
    loop {
        let Some(header) = buffer.get(offset..offset + 4) else {
            return Ok(None);
        };
        let length = std::str::from_utf8(header)
            .ok()
            .and_then(|header| usize::from_str_radix(header, 16).ok())
            .ok_or_else(invalid)?;
        if length == 0 {
            return Ok(Some(refs));
        }
        if length <= 4 {
            return Err(invalid());
        }
        let Some(payload) = buffer.get(offset + 4..offset + length) else {
            return Ok(None);
        };
        offset += length;
        let line = payload.split(|byte| *byte == 0).next().unwrap_or(payload);
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        if line.starts_with(b"shallow ") {
            continue;
        }
        let mut fields = line.splitn(3, |byte| *byte == b' ');
        let (Some(_), Some(_), Some(refname)) = (fields.next(), fields.next(), fields.next())
        else {
            return Err(invalid());
        };
        refs.push(String::from_utf8(refname.to_vec()).map_err(|_| invalid())?);
    }
}

pub(crate) fn advertise_additional_refs(
    advertisement: &[u8],
    refs: &[(String, String)],
) -> Result<Vec<u8>, ApiError> {
    let invalid = || ApiError::internal_message("unexpected Git receive-pack advertisement");
    if refs.is_empty() {
        return Ok(advertisement.to_vec());
    }
    let body = advertisement.strip_suffix(FLUSH).ok_or_else(invalid)?;
    let mut lines = Vec::new();
    let mut offset = 0;
    while offset < body.len() {
        let header = body.get(offset..offset + 4).ok_or_else(invalid)?;
        let length = std::str::from_utf8(header)
            .ok()
            .and_then(|header| usize::from_str_radix(header, 16).ok())
            .ok_or_else(invalid)?;
        let end = if length == 0 {
            offset + 4
        } else {
            offset + length
        };
        lines.push(body.get(offset..end).ok_or_else(invalid)?);
        offset = end;
    }
    let first_ref = lines
        .iter()
        .position(|line| *line == FLUSH)
        .map(|flush| flush + 1)
        .unwrap_or(0);
    let mut output = Vec::with_capacity(advertisement.len() + refs.len() * 96);
    let mut extra = refs.iter();
    for (index, line) in lines.iter().enumerate() {
        if index == first_ref && line.get(4..).is_some_and(is_no_refs_line) {
            let (refname, oid) = extra.next().ok_or_else(invalid)?;
            let capabilities = line
                .iter()
                .position(|byte| *byte == 0)
                .map(|nul| &line[nul..])
                .ok_or_else(invalid)?;
            let mut payload = format!("{oid} {refname}").into_bytes();
            payload.extend_from_slice(capabilities);
            output.extend(pkt_line(&payload));
        } else {
            output.extend_from_slice(line);
        }
    }
    for (refname, oid) in extra {
        output.extend(pkt_line(format!("{oid} {refname}\n").as_bytes()));
    }
    output.extend_from_slice(FLUSH);
    Ok(output)
}

fn is_no_refs_line(payload: &[u8]) -> bool {
    payload
        .split(|byte| *byte == 0)
        .next()
        .and_then(|line| line.split(|byte| *byte == b' ').nth(1))
        .is_some_and(|refname| refname == NO_REFS.as_bytes())
}

#[cfg(test)]
mod tests;
