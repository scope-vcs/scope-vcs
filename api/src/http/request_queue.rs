use super::{
    requests::{current_main_oid_for_context, repo_metadata_and_access},
    responses::{request_actor_summary_response, request_list_item_response},
};
use crate::auth::scope::require_scope_user;
use crate::repo_events::RepoChangeReason;
use crate::{error::ApiError, state::AppState};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use scope_api_contract::{
    RequestAttentionActionRequest, RequestAttentionMutationResponse, RequestAttentionResponse,
    RequestQueueItemResponse, RequestQueuePageResponse,
};
use scope_domain::requests::{
    REQUEST_LIST_DEFAULT_PAGE_SIZE, REQUEST_LIST_MAX_PAGE_SIZE, RequestQueueClassification,
    RequestQueueSection,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const CURSOR_PREFIX: &str = "scope_rq_";
const CURSOR_NONCE_BYTES: usize = 12;
const CURSOR_MAX_ENCODED_BYTES: usize = 2_048;
const CURSOR_KEY_DOMAIN: &[u8] = b"scope.request-queue-cursor.key.v2\0";
const CURSOR_AAD_DOMAIN: &str = "scope.request-queue-cursor.aad.v2";
const SEARCH_MAX_CHARS: usize = 200;

#[derive(Debug, Deserialize)]
pub(crate) struct RequestQueueQuery {
    section: RequestQueueSection,
    cursor: Option<String>,
    limit: Option<usize>,
    search: Option<String>,
}

pub(crate) async fn request_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<RequestQueueQuery>,
) -> Result<Json<RequestQueuePageResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    let after = query
        .cursor
        .as_deref()
        .map(|cursor| {
            parse_cursor(
                state.push_intent_signing_key.as_ref(),
                &repo.record.id,
                query.section,
                cursor,
            )
        })
        .transpose()?;
    let search = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|search| !search.is_empty());
    if search.is_some_and(|search| search.chars().count() > SEARCH_MAX_CHARS) {
        return Err(ApiError::bad_request("request queue search is too long"));
    }
    let limit = query
        .limit
        .unwrap_or(REQUEST_LIST_DEFAULT_PAGE_SIZE)
        .clamp(1, REQUEST_LIST_MAX_PAGE_SIZE);
    let now_unix = crate::persistence::unix_now()?;
    let mut page = state
        .metadata
        .requests()
        .request_queue_page(scope_postgres::db::RequestQueuePageQuery {
            repo_id: &repo.record.id,
            section: query.section,
            viewer_user_id: viewer_user_id.as_deref(),
            access,
            search,
            after: after.as_ref(),
            limit: (limit + 1) as u64,
            now_unix,
        })
        .await?;
    let has_more = page.rows.len() > limit;
    page.rows.truncate(limit);
    let next_cursor = if has_more {
        page.rows
            .last()
            .map(|row| {
                encode_cursor(
                    state.push_intent_signing_key.as_ref(),
                    &repo.record.id,
                    query.section,
                    &row.cursor,
                )
            })
            .transpose()?
    } else {
        None
    };
    let current_main_oid = if page.rows.is_empty() {
        None
    } else {
        current_main_oid_for_context(&state, &repo).await?
    };
    let requests = page
        .rows
        .into_iter()
        .map(|row| {
            let author = request_actor_summary_response(&row.request.author_user_id, &page.users)?;
            let claimer = row
                .claim
                .as_ref()
                .map(|claim| request_actor_summary_response(&claim.claimer_user_id, &page.users))
                .transpose()?;
            let activity_version = row.request.activity_version;
            Ok(RequestQueueItemResponse {
                attention_at_unix: row.cursor.updated_at_unix,
                request: request_list_item_response(row.request, access, current_main_oid.clone())?,
                author,
                attention: attention_response(row.attention, activity_version),
                claimer,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(RequestQueuePageResponse {
        requests,
        next_cursor,
        next_attention_at_unix: page.next_attention_at_unix,
    }))
}

pub(crate) async fn apply_attention(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<RequestAttentionActionRequest>,
) -> Result<Json<RequestAttentionMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, _, _) = repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    let (action, expected_activity_version) = match input {
        RequestAttentionActionRequest::Claim {
            expected_activity_version,
        } => (
            scope_domain::requests::RequestAttentionAction::Claim,
            expected_activity_version,
        ),
        RequestAttentionActionRequest::Wait {
            expected_activity_version,
        } => (
            scope_domain::requests::RequestAttentionAction::Wait,
            expected_activity_version,
        ),
        RequestAttentionActionRequest::Settle {
            expected_activity_version,
        } => (
            scope_domain::requests::RequestAttentionAction::Settle,
            expected_activity_version,
        ),
        RequestAttentionActionRequest::Snooze {
            expected_activity_version,
            until_unix,
        } => (
            scope_domain::requests::RequestAttentionAction::Snooze { until_unix },
            expected_activity_version,
        ),
        RequestAttentionActionRequest::Restore {
            expected_activity_version,
        } => (
            scope_domain::requests::RequestAttentionAction::Restore,
            expected_activity_version,
        ),
        RequestAttentionActionRequest::Release {
            expected_activity_version,
        } => (
            scope_domain::requests::RequestAttentionAction::Release,
            expected_activity_version,
        ),
    };
    let result = state
        .metadata
        .requests()
        .apply_request_attention(scope_postgres::db::ApplyRequestAttentionCommand {
            repo_id: repo.record.id.clone(),
            request_id,
            actor_user_id: user.id,
            expected_activity_version,
            action,
            now_unix: crate::persistence::unix_now()?,
        })
        .await?;
    state
        .publish_request_summary_refresh(
            &repo.incarnation(),
            RepoChangeReason::RequestAttentionChanged,
        )
        .await;
    let claimer = if let Some(claim) = &result.claim {
        let users = state
            .metadata
            .auth()
            .users_by_ids([claim.claimer_user_id.clone()])
            .await?;
        Some(request_actor_summary_response(
            &claim.claimer_user_id,
            &users,
        )?)
    } else {
        None
    };
    Ok(Json(RequestAttentionMutationResponse {
        attention: attention_response(result.attention, result.activity_version),
        claimer,
    }))
}

pub(crate) fn attention_response(
    value: RequestQueueClassification,
    activity_version: u64,
) -> RequestAttentionResponse {
    RequestAttentionResponse {
        state: value.state.into(),
        reason: value.reason.into(),
        activity_version,
        through_activity_version: value.through_activity_version,
        snoozed_until_unix: value.snoozed_until_unix,
        can_claim: value.can_claim,
        can_set_aside: value.can_set_aside,
        can_restore: value.can_restore,
        can_release: value.can_release,
    }
}

fn encode_cursor(
    signing_key: &[u8],
    repo_id: &str,
    section: RequestQueueSection,
    cursor: &scope_postgres::db::RequestQueueCursor,
) -> Result<String, ApiError> {
    let plaintext = format!("{}:{}", cursor.updated_at_unix, cursor.request_id);
    let mut nonce = [0_u8; CURSOR_NONCE_BYTES];
    getrandom::fill(&mut nonce).map_err(|error| {
        ApiError::internal_message(format!("queue cursor nonce failed: {error}"))
    })?;
    let ciphertext = ChaCha20Poly1305::new(Key::from_slice(&cursor_key(signing_key)))
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: cursor_aad(repo_id, section).as_bytes(),
            },
        )
        .map_err(|_| ApiError::internal_message("queue cursor encryption failed"))?;
    let mut envelope = Vec::with_capacity(nonce.len() + ciphertext.len());
    envelope.extend_from_slice(&nonce);
    envelope.extend_from_slice(&ciphertext);
    Ok(format!(
        "{CURSOR_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(envelope)
    ))
}

fn parse_cursor(
    signing_key: &[u8],
    repo_id: &str,
    section: RequestQueueSection,
    value: &str,
) -> Result<scope_postgres::db::RequestQueueCursor, ApiError> {
    let invalid = || ApiError::bad_request("invalid request queue cursor");
    let encoded = value
        .strip_prefix(CURSOR_PREFIX)
        .filter(|encoded| encoded.len() <= CURSOR_MAX_ENCODED_BYTES)
        .ok_or_else(invalid)?;
    let envelope = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?;
    if envelope.len() <= CURSOR_NONCE_BYTES {
        return Err(invalid());
    }
    let (nonce, ciphertext) = envelope.split_at(CURSOR_NONCE_BYTES);
    let plaintext = ChaCha20Poly1305::new(Key::from_slice(&cursor_key(signing_key)))
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: cursor_aad(repo_id, section).as_bytes(),
            },
        )
        .map_err(|_| invalid())?;
    let plaintext = std::str::from_utf8(&plaintext).map_err(|_| invalid())?;
    let (updated, request_id) = plaintext.split_once(':').ok_or_else(invalid)?;
    let updated_at_unix = updated.parse::<u64>().map_err(|_| invalid())?;
    i64::try_from(updated_at_unix).map_err(|_| invalid())?;
    if request_id.is_empty() || request_id.contains(':') {
        return Err(invalid());
    }
    Ok(scope_postgres::db::RequestQueueCursor {
        updated_at_unix,
        request_id: request_id.to_string(),
    })
}

fn cursor_key(signing_key: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(CURSOR_KEY_DOMAIN);
    digest.update(signing_key);
    digest.finalize().into()
}

fn cursor_aad(repo_id: &str, section: RequestQueueSection) -> String {
    let section = section.as_str();
    format!("{CURSOR_AAD_DOMAIN}\0{repo_id}\0{section}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_cursor_is_confidential_and_bound_to_section() {
        let cursor = scope_postgres::db::RequestQueueCursor {
            updated_at_unix: 12,
            request_id: "request".into(),
        };
        let encoded = encode_cursor(b"key", "repo", RequestQueueSection::Active, &cursor).unwrap();
        assert!(!encoded.contains("request"));
        assert_eq!(
            parse_cursor(b"key", "repo", RequestQueueSection::Active, &encoded).unwrap(),
            cursor
        );
        assert!(parse_cursor(b"key", "repo", RequestQueueSection::SetAside, &encoded).is_err());
        let out_of_range = scope_postgres::db::RequestQueueCursor {
            updated_at_unix: i64::MAX as u64 + 1,
            request_id: "request".into(),
        };
        let encoded =
            encode_cursor(b"key", "repo", RequestQueueSection::Active, &out_of_range).unwrap();
        assert!(parse_cursor(b"key", "repo", RequestQueueSection::Active, &encoded).is_err());
    }
}
