use super::*;

pub(super) const TEST_CLERK_ISSUER: &str = "https://clerk.test";
pub(super) const TEST_CLERK_AUDIENCE: &str = "scope-api";
pub(super) const TEST_CLERK_USER_ID: &str = "user_owner";
pub(super) const TEST_OWNER_EMAIL: &str = "owner@example.com";

pub(super) const TEST_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgj30p9gYDpHRqbshS
LyBNueRnRb9WS031zFD7yuhqn/ChRANCAAR6wR8PANHsn10BAVi085aM8LBPL3Cj
kGxvBjzgF9RjXJoldYnFk7mJ5gLANHjaaad3qTQJ8DldKJoSqkEkm5gg
-----END PRIVATE KEY-----"#;

pub(super) const TEST_JWKS: &str = r#"{
  "keys": [{
    "kty": "EC",
    "x": "esEfDwDR7J9dAQFYtPOWjPCwTy9wo5BsbwY84BfUY1w",
    "y": "miV1icWTuYnmAsA0eNppp3epNAnwOV0omhKqQSSbmCA",
    "crv": "P-256",
    "kid": "test-key",
    "use": "sig",
    "alg": "ES256"
  }]
}"#;

pub(super) fn test_jwks() -> JwkSet {
    serde_json::from_str(TEST_JWKS).unwrap()
}

pub(super) fn sign_claims(claims: serde_json::Value) -> String {
    sign_claims_with_kid(claims, "test-key")
}

pub(super) fn sign_claims_with_kid(claims: serde_json::Value, kid: &str) -> String {
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(kid.into());
    encode(
        &header,
        &claims,
        &EncodingKey::from_ec_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap(),
    )
    .unwrap()
}

pub(super) fn token(user_id: &str, email_verified: bool) -> String {
    token_for_claims(
        user_id,
        Some(TEST_OWNER_EMAIL.to_string()),
        email_verified,
        Some(LOCAL_APP_ORIGIN),
        None,
    )
}

pub(super) fn token_with_audience(user_id: &str, aud: serde_json::Value) -> String {
    token_for_claims(
        user_id,
        Some(TEST_OWNER_EMAIL.to_string()),
        true,
        Some(LOCAL_APP_ORIGIN),
        Some(aud),
    )
}

pub(super) fn token_for_claims(
    user_id: &str,
    email: Option<String>,
    email_verified: bool,
    azp: Option<&str>,
    aud: Option<serde_json::Value>,
) -> String {
    let mut claims = serde_json::json!({
        "iss": TEST_CLERK_ISSUER,
        "exp": unix_now() + 300,
        "sub": user_id,
        "email": email,
        "email_verified": email_verified,
    });
    if let Some(azp) = azp {
        claims["azp"] = serde_json::json!(azp);
    }
    if let Some(aud) = aud {
        claims["aud"] = aud;
    }

    sign_claims(claims)
}

pub(super) fn test_clerk_policy() -> ClerkTokenPolicy {
    ClerkTokenPolicy {
        authorized_parties: vec![LOCAL_APP_ORIGIN.to_string()],
        audiences: vec![TEST_CLERK_AUDIENCE.to_string()],
    }
}

pub(super) fn cache_test_jwks(state: &AppState) {
    state.clerk.cache_jwks_for_tests(test_jwks());
}

pub(super) fn bearer_header() -> String {
    format!("Bearer {}", api_token(TEST_CLERK_USER_ID, TEST_OWNER_EMAIL))
}

pub(super) fn bearer_header_for(user_id: &str, email: &str) -> String {
    format!("Bearer {}", api_token(user_id, email))
}

pub(super) fn api_token(user_id: &str, email: &str) -> String {
    token_for_claims(
        user_id,
        Some(email.to_string()),
        true,
        Some(LOCAL_APP_ORIGIN),
        Some(serde_json::json!(TEST_CLERK_AUDIENCE)),
    )
}
