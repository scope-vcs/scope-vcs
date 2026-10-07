use crate::{error::ApiError, persistence::unix_now, state::AppState};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use scope_domain::{
    repo_config::{RepoConfig, repo_config_fingerprint as domain_repo_config_fingerprint},
    repository::git::GitFrontier,
    views::ViewId,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::Arc;

const PUSH_INTENT_TTL_SECS: u64 = 10 * 60;
const PUSH_INTENT_TOKEN_PREFIX: &str = "scope_pi_";
const PUSH_INTENT_KIND: &str = "scope.push-intent";
const PUSH_INTENT_KEY_DERIVATION_CONTEXT: &[u8] = b"scope.push-intent.signing-key.v1";
type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PushIntentTarget {
    Canonical {
        config: RepoConfig,
        base_config_hash: String,
        base_git_frontier: Option<GitFrontier>,
    },
    View {
        view: ViewId,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PushIntentClaims {
    kind: String,
    repo_id: String,
    user_id: String,
    head_oid: String,
    target: PushIntentTarget,
    expires_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValidatedPushIntent {
    pub(crate) repo_id: String,
    pub(crate) user_id: String,
    pub(crate) head_oid: String,
    pub(crate) target: PushIntentTarget,
    pub(crate) expires_at_unix: u64,
}

pub(crate) struct CanonicalPushIntent<'a> {
    pub(crate) config: &'a RepoConfig,
    pub(crate) base_config_hash: &'a str,
    base_git_frontier: &'a Option<GitFrontier>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CreatedPushIntent {
    pub(crate) token: String,
    pub(crate) expires_at_unix: u64,
}

impl ValidatedPushIntent {
    pub(crate) fn ensure_repo_user(&self, repo_id: &str, user_id: &str) -> Result<(), ApiError> {
        if self.repo_id == repo_id && self.user_id == user_id {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "Scope push intent does not match received Git push",
            ))
        }
    }

    pub(crate) fn ensure_head(&self, head_oid: &str) -> Result<(), ApiError> {
        if self.head_oid == head_oid {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "Scope push intent does not match received Git push",
            ))
        }
    }

    pub(crate) fn canonical(&self) -> Result<CanonicalPushIntent<'_>, ApiError> {
        match &self.target {
            PushIntentTarget::Canonical {
                config,
                base_config_hash,
                base_git_frontier,
            } => Ok(CanonicalPushIntent {
                config,
                base_config_hash,
                base_git_frontier,
            }),
            PushIntentTarget::View { view } => Err(ApiError::forbidden(format!(
                "Scope push intent goes through the {view} view, not the full view"
            ))),
        }
    }

    pub(crate) fn through_view(&self) -> Option<&ViewId> {
        match &self.target {
            PushIntentTarget::View { view } => Some(view),
            PushIntentTarget::Canonical { .. } => None,
        }
    }
}

impl CanonicalPushIntent<'_> {
    pub(crate) fn base_for_head(
        &self,
        intent: &ValidatedPushIntent,
        head_oid: &str,
    ) -> Result<Option<GitFrontier>, ApiError> {
        intent.ensure_head(head_oid)?;
        Ok(self.base_git_frontier.clone())
    }
}

impl AppState {
    pub(crate) fn create_push_intent(
        &self,
        repo_id: &str,
        user_id: &str,
        head_oid: &str,
        target: PushIntentTarget,
    ) -> Result<CreatedPushIntent, ApiError> {
        let expires_at_unix = unix_now()?.saturating_add(PUSH_INTENT_TTL_SECS);
        let intent = PushIntentClaims {
            kind: PUSH_INTENT_KIND.to_string(),
            repo_id: repo_id.to_string(),
            user_id: user_id.to_string(),
            head_oid: head_oid.to_string(),
            target,
            expires_at_unix,
        };
        let token = encode_push_intent(&self.push_intent_signing_key, &intent)?;
        Ok(CreatedPushIntent {
            token,
            expires_at_unix,
        })
    }

    pub(crate) fn validate_push_intent_secret(
        &self,
        secret: &str,
    ) -> Result<ValidatedPushIntent, ApiError> {
        decode_push_intent(&self.push_intent_signing_key, secret)
            .map(validated_push_intent_from_claims)
    }
}

pub(crate) fn push_intent_signing_key(shared_root_key: &[u8]) -> Result<Arc<[u8]>, ApiError> {
    let mut mac = HmacSha256::new_from_slice(shared_root_key).map_err(ApiError::internal)?;
    mac.update(PUSH_INTENT_KEY_DERIVATION_CONTEXT);
    Ok(Arc::from(mac.finalize().into_bytes().to_vec()))
}

pub(crate) fn repo_config_fingerprint(config: &RepoConfig) -> Result<String, ApiError> {
    domain_repo_config_fingerprint(config).map_err(ApiError::internal)
}

fn encode_push_intent(signing_key: &[u8], intent: &PushIntentClaims) -> Result<String, ApiError> {
    let payload = serde_json::to_vec(intent).map_err(ApiError::internal)?;
    let payload = URL_SAFE_NO_PAD.encode(payload);
    let signature = sign_push_intent(signing_key, payload.as_bytes())?;
    Ok(format!(
        "{PUSH_INTENT_TOKEN_PREFIX}{payload}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn validated_push_intent_from_claims(intent: PushIntentClaims) -> ValidatedPushIntent {
    ValidatedPushIntent {
        repo_id: intent.repo_id,
        user_id: intent.user_id,
        head_oid: intent.head_oid,
        target: intent.target,
        expires_at_unix: intent.expires_at_unix,
    }
}

fn decode_push_intent(signing_key: &[u8], token: &str) -> Result<PushIntentClaims, ApiError> {
    let Some(token) = token.trim().strip_prefix(PUSH_INTENT_TOKEN_PREFIX) else {
        return Err(ApiError::forbidden("valid Scope push intent required"));
    };
    let Some((payload, signature)) = token.split_once('.') else {
        return Err(ApiError::forbidden("valid Scope push intent required"));
    };
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))?;
    verify_push_intent_signature(signing_key, payload.as_bytes(), &signature)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))?;
    let intent: PushIntentClaims = serde_json::from_slice(&payload)
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))?;
    if intent.kind != PUSH_INTENT_KIND {
        return Err(ApiError::forbidden("valid Scope push intent required"));
    }
    if intent.expires_at_unix <= unix_now()? {
        return Err(ApiError::forbidden("valid Scope push intent required"));
    }
    Ok(intent)
}

fn sign_push_intent(signing_key: &[u8], payload: &[u8]) -> Result<Vec<u8>, ApiError> {
    let mut mac = HmacSha256::new_from_slice(signing_key).map_err(ApiError::internal)?;
    mac.update(payload);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn verify_push_intent_signature(
    signing_key: &[u8],
    payload: &[u8],
    signature: &[u8],
) -> Result<(), ApiError> {
    let mut mac = HmacSha256::new_from_slice(signing_key).map_err(ApiError::internal)?;
    mac.update(payload);
    mac.verify_slice(signature)
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_push_intents_encode_the_frontier_directly() {
        let key = [7_u8; 32];
        let digest = "b".repeat(64);
        let config = RepoConfig::with_default_view(scope_domain::views::ViewId::private());
        let config = serde_json::to_string(&config).unwrap();
        let payload = format!(
            r#"{{"kind":"scope.push-intent","repo_id":"owner/repo","user_id":"owner","head_oid":"next-head","target":{{"canonical":{{"config":{config},"base_config_hash":"config-hash","base_git_frontier":"{digest}"}}}},"expires_at_unix":4000000000}}"#
        );
        let encoded = URL_SAFE_NO_PAD.encode(payload);
        let signature = sign_push_intent(&key, encoded.as_bytes()).unwrap();
        let token = format!("scope_pi_{encoded}.{}", URL_SAFE_NO_PAD.encode(signature));
        let claims = decode_push_intent(&key, &token).unwrap();
        assert_eq!(encode_push_intent(&key, &claims).unwrap(), token);
        let validated = validated_push_intent_from_claims(claims);
        let canonical = validated.canonical().unwrap();
        assert_eq!(
            canonical.base_for_head(&validated, "next-head").unwrap(),
            Some(GitFrontier::from_digest(digest)),
        );
        assert!(
            canonical
                .base_for_head(&validated, "different-head")
                .is_err()
        );
    }

    #[test]
    fn view_push_intents_carry_only_their_view() {
        let key = [7_u8; 32];
        let agent = ViewId::parse("agent").unwrap();
        let claims = PushIntentClaims {
            kind: PUSH_INTENT_KIND.to_string(),
            repo_id: "owner/repo".to_string(),
            user_id: "member".to_string(),
            head_oid: "next-head".to_string(),
            target: PushIntentTarget::View {
                view: agent.clone(),
            },
            expires_at_unix: 4_000_000_000,
        };
        let token = encode_push_intent(&key, &claims).unwrap();
        let payload = token
            .strip_prefix(PUSH_INTENT_TOKEN_PREFIX)
            .and_then(|token| token.split_once('.'))
            .map(|(payload, _)| URL_SAFE_NO_PAD.decode(payload).unwrap())
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&payload).unwrap()["target"],
            serde_json::json!({"view": {"view": "agent"}})
        );
        let validated =
            validated_push_intent_from_claims(decode_push_intent(&key, &token).unwrap());
        assert_eq!(validated.through_view(), Some(&agent));
        assert!(validated.canonical().is_err());
    }

    #[test]
    fn shared_root_key_derives_one_domain_separated_signing_key() {
        let root = [7_u8; 32];
        let first = push_intent_signing_key(&root).unwrap();
        let second = push_intent_signing_key(&root).unwrap();
        let other = push_intent_signing_key(&[8_u8; 32]).unwrap();

        assert_eq!(first.as_ref(), second.as_ref());
        assert_ne!(first.as_ref(), root.as_slice());
        assert_ne!(first.as_ref(), other.as_ref());
    }
}
