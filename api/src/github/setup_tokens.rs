//! Signed, expiring proofs for the connect flow. GitHub passes the install
//! state back unchanged, so it names who started the flow and for which
//! repository; an installation id arriving with it is never trusted on its
//! own. The connect grant then carries what the signed-in GitHub account was
//! shown to reach, so the user token never needs to be stored.

use crate::error::ApiError;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::Sha256;

/// Long enough for GitHub's install screen and a repository choice.
pub(crate) const GITHUB_SETUP_TTL_SECS: u64 = 10 * 60;
const INSTALL_STATE_KIND: &str = "scope.github-install";
const CONNECT_GRANT_KIND: &str = "scope.github-connect";
const KEY_DERIVATION_CONTEXT: &[u8] = b"scope.github-setup.signing-key.v1";
const INVALID_SETUP: &str =
    "This GitHub setup expired or is not valid. Start again from repository settings.";
type HmacSha256 = Hmac<Sha256>;

/// Who started connecting which Scope repository.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct InstallState {
    kind: String,
    pub(crate) owner: String,
    pub(crate) repo: String,
    pub(crate) user_id: String,
    expires_at_unix: u64,
}

/// What one signed-in GitHub account can reach through one installation.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct ConnectGrant {
    kind: String,
    pub(crate) owner: String,
    pub(crate) repo: String,
    pub(crate) user_id: String,
    pub(crate) installation_id: u64,
    pub(crate) repository_ids: Vec<u64>,
    expires_at_unix: u64,
}

pub(crate) struct SetupTokenSigner {
    key: Vec<u8>,
}

impl SetupTokenSigner {
    /// The key is derived from the app's client secret, so rotating that
    /// secret also ends flows already in progress.
    pub(crate) fn from_client_secret(client_secret: &str) -> Self {
        let mut mac = HmacSha256::new_from_slice(client_secret.as_bytes())
            .expect("HMAC accepts keys of any length");
        mac.update(KEY_DERIVATION_CONTEXT);
        Self {
            key: mac.finalize().into_bytes().to_vec(),
        }
    }

    pub(crate) fn install_state(&self, owner: &str, repo: &str, user_id: &str, now: u64) -> String {
        self.sign(&InstallState {
            kind: INSTALL_STATE_KIND.to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
            user_id: user_id.to_string(),
            expires_at_unix: now.saturating_add(GITHUB_SETUP_TTL_SECS),
        })
    }

    pub(crate) fn open_install_state(
        &self,
        token: &str,
        now: u64,
    ) -> Result<InstallState, ApiError> {
        self.open::<InstallState>(token)
            .filter(|state| state.kind == INSTALL_STATE_KIND && now < state.expires_at_unix)
            .ok_or_else(|| ApiError::forbidden(INVALID_SETUP))
    }

    pub(crate) fn connect_grant(
        &self,
        state: &InstallState,
        installation_id: u64,
        repository_ids: Vec<u64>,
        now: u64,
    ) -> String {
        self.sign(&ConnectGrant {
            kind: CONNECT_GRANT_KIND.to_string(),
            owner: state.owner.clone(),
            repo: state.repo.clone(),
            user_id: state.user_id.clone(),
            installation_id,
            repository_ids,
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
    fn install_state_round_trips_until_it_expires() {
        let signer = SetupTokenSigner::from_client_secret("client-secret");
        let token = signer.install_state("owner", "repo", "user_owner", NOW);
        let state = signer.open_install_state(&token, NOW + 1).unwrap();
        assert_eq!(
            (state.owner.as_str(), state.repo.as_str()),
            ("owner", "repo")
        );
        assert_eq!(state.user_id, "user_owner");

        assert!(
            signer
                .open_install_state(&token, NOW + GITHUB_SETUP_TTL_SECS)
                .is_err()
        );
    }

    #[test]
    fn tampered_or_foreign_tokens_are_rejected() {
        let signer = SetupTokenSigner::from_client_secret("client-secret");
        let token = signer.install_state("owner", "repo", "user_owner", NOW);
        let other = SetupTokenSigner::from_client_secret("rotated-secret");
        assert!(other.open_install_state(&token, NOW).is_err());

        let (payload, signature) = token.split_once('.').unwrap();
        let forged_claims = String::from_utf8(URL_SAFE_NO_PAD.decode(payload).unwrap())
            .unwrap()
            .replace("user_owner", "user_attacker");
        let forged = format!("{}.{signature}", URL_SAFE_NO_PAD.encode(forged_claims));
        assert!(signer.open_install_state(&forged, NOW).is_err());
        assert!(signer.open_install_state("not-a-token", NOW).is_err());
    }

    #[test]
    fn install_state_and_connect_grant_are_not_interchangeable() {
        let signer = SetupTokenSigner::from_client_secret("client-secret");
        let state_token = signer.install_state("owner", "repo", "user_owner", NOW);
        let state = signer.open_install_state(&state_token, NOW).unwrap();
        let grant_token = signer.connect_grant(&state, 7, vec![42], NOW);

        assert!(signer.open_connect_grant(&state_token, NOW).is_err());
        assert!(signer.open_install_state(&grant_token, NOW).is_err());
        let grant = signer.open_connect_grant(&grant_token, NOW).unwrap();
        assert_eq!(grant.installation_id, 7);
        assert_eq!(grant.repository_ids, vec![42]);
    }
}
