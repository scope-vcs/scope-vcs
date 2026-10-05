use crate::error::ApiError;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::Sha256;

pub(crate) const GITHUB_SETUP_TTL_SECS: u64 = 10 * 60;
const SETUP_STATE_KIND: &str = "scope.github-setup";
const CONNECT_GRANT_KIND: &str = "scope.github-connect";
const KEY_DERIVATION_CONTEXT: &[u8] = b"scope.github-setup.signing-key.v1";
const INVALID_SETUP: &str =
    "This GitHub setup expired or is not valid. Start again from repository settings.";
type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct SetupState {
    kind: String,
    pub(crate) owner: String,
    pub(crate) repo: String,
    pub(crate) user_id: String,
    expires_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct ConnectGrant {
    kind: String,
    pub(crate) owner: String,
    pub(crate) repo: String,
    pub(crate) user_id: String,
    repositories: Vec<GrantedRepository>,
    expires_at_unix: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct GrantedRepository {
    pub(crate) id: u64,
    pub(crate) installation_id: u64,
}

impl ConnectGrant {
    pub(crate) fn installation_for(&self, repository_id: u64) -> Option<u64> {
        self.repositories
            .iter()
            .find(|repository| repository.id == repository_id)
            .map(|repository| repository.installation_id)
    }
}

pub(crate) struct SetupTokenSigner {
    key: Vec<u8>,
}

impl SetupTokenSigner {
    pub(crate) fn from_client_secret(client_secret: &str) -> Self {
        let mut mac = HmacSha256::new_from_slice(client_secret.as_bytes())
            .expect("HMAC accepts keys of any length");
        mac.update(KEY_DERIVATION_CONTEXT);
        Self {
            key: mac.finalize().into_bytes().to_vec(),
        }
    }

    pub(crate) fn setup_state(&self, owner: &str, repo: &str, user_id: &str, now: u64) -> String {
        self.sign(&SetupState {
            kind: SETUP_STATE_KIND.to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
            user_id: user_id.to_string(),
            expires_at_unix: now.saturating_add(GITHUB_SETUP_TTL_SECS),
        })
    }

    pub(crate) fn open_setup_state(&self, token: &str, now: u64) -> Result<SetupState, ApiError> {
        self.open::<SetupState>(token)
            .filter(|state| state.kind == SETUP_STATE_KIND && now < state.expires_at_unix)
            .ok_or_else(|| ApiError::forbidden(INVALID_SETUP))
    }

    pub(crate) fn connect_grant(
        &self,
        state: &SetupState,
        repositories: Vec<GrantedRepository>,
        now: u64,
    ) -> String {
        self.sign(&ConnectGrant {
            kind: CONNECT_GRANT_KIND.to_string(),
            owner: state.owner.clone(),
            repo: state.repo.clone(),
            user_id: state.user_id.clone(),
            repositories,
            expires_at_unix: now.saturating_add(GITHUB_SETUP_TTL_SECS),
        })
    }

    pub(crate) fn open_connect_grant(
        &self,
        token: &str,
        now: u64,
    ) -> Result<ConnectGrant, ApiError> {
        self.open::<ConnectGrant>(token)
            .filter(|grant| grant.kind == CONNECT_GRANT_KIND && now < grant.expires_at_unix)
            .ok_or_else(|| ApiError::forbidden(INVALID_SETUP))
    }

    fn sign<T: Serialize>(&self, claims: &T) -> String {
        let payload = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(claims).expect("setup claims serialize as JSON"));
        let signature =
            URL_SAFE_NO_PAD.encode(self.mac(payload.as_bytes()).finalize().into_bytes());
        format!("{payload}.{signature}")
    }

    fn open<T: DeserializeOwned>(&self, token: &str) -> Option<T> {
        let (payload, signature) = token.trim().split_once('.')?;
        let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
        self.mac(payload.as_bytes()).verify_slice(&signature).ok()?;
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()
    }

    fn mac(&self, payload: &[u8]) -> HmacSha256 {
        let mut mac =
            HmacSha256::new_from_slice(&self.key).expect("HMAC accepts keys of any length");
        mac.update(payload);
        mac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000;

    #[test]
    fn setup_state_round_trips_until_it_expires() {
        let signer = SetupTokenSigner::from_client_secret("client-secret");
        let token = signer.setup_state("owner", "repo", "user_owner", NOW);
        let state = signer.open_setup_state(&token, NOW + 1).unwrap();
        assert_eq!(
            (state.owner.as_str(), state.repo.as_str()),
            ("owner", "repo")
        );
        assert_eq!(state.user_id, "user_owner");

        assert!(
            signer
                .open_setup_state(&token, NOW + GITHUB_SETUP_TTL_SECS)
                .is_err()
        );
    }

    #[test]
    fn tampered_or_foreign_tokens_are_rejected() {
        let signer = SetupTokenSigner::from_client_secret("client-secret");
        let token = signer.setup_state("owner", "repo", "user_owner", NOW);
        let other = SetupTokenSigner::from_client_secret("rotated-secret");
        assert!(other.open_setup_state(&token, NOW).is_err());

        let (payload, signature) = token.split_once('.').unwrap();
        let forged_claims = String::from_utf8(URL_SAFE_NO_PAD.decode(payload).unwrap())
            .unwrap()
            .replace("user_owner", "user_attacker");
        let forged = format!("{}.{signature}", URL_SAFE_NO_PAD.encode(forged_claims));
        assert!(signer.open_setup_state(&forged, NOW).is_err());
        assert!(signer.open_setup_state("not-a-token", NOW).is_err());
    }

    #[test]
    fn setup_state_and_connect_grant_are_not_interchangeable() {
        let signer = SetupTokenSigner::from_client_secret("client-secret");
        let state_token = signer.setup_state("owner", "repo", "user_owner", NOW);
        let state = signer.open_setup_state(&state_token, NOW).unwrap();
        let granted = GrantedRepository {
            id: 42,
            installation_id: 7,
        };
        let grant_token = signer.connect_grant(&state, vec![granted], NOW);

        assert!(signer.open_connect_grant(&state_token, NOW).is_err());
        assert!(signer.open_setup_state(&grant_token, NOW).is_err());
        let grant = signer.open_connect_grant(&grant_token, NOW).unwrap();
        assert_eq!(grant.installation_for(42), Some(7));
        assert_eq!(grant.installation_for(43), None);
    }
}
