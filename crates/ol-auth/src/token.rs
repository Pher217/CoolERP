use chrono::Utc;
use jsonwebtoken::{Algorithm, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AuthError, Result};
use crate::keys::SigningKeys;

/// Standard JWT claims used by the CoolERP OAuth 2.1 AS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenClaims {
    /// Issuer (`iss`).
    pub iss: String,
    /// Subject — typically the client_id or user identifier (`sub`).
    pub sub: String,
    /// Audience (`aud`).
    pub aud: Vec<String>,
    /// Expiry as a Unix timestamp in seconds (`exp`).
    pub exp: usize,
    /// Issued-at as a Unix timestamp in seconds (`iat`).
    pub iat: usize,
    /// Unique JWT identifier (`jti`).
    pub jti: String,
    /// Space-separated list of granted OAuth 2.1 scopes (`scope`).
    pub scope: String,
}

impl TokenClaims {
    /// Parse the `scope` field into individual scope strings.
    pub fn scopes(&self) -> Vec<&str> {
        self.scope.split_whitespace().collect()
    }

    /// Return `true` if `s` is present in the space-separated scope list.
    pub fn has_scope(&self, s: &str) -> bool {
        self.scope.split_whitespace().any(|sc| sc == s)
    }
}

/// Mint a signed JWT access token.
///
/// - `keys`     — the active signing key pair
/// - `issuer`   — value for the `iss` claim (e.g. `"https://auth.example.com"`)
/// - `sub`      — subject identifier
/// - `audience` — value for the `aud` claim
/// - `scopes`   — granted OAuth 2.1 scopes; joined with a space
/// - `ttl`      — token lifetime; use a negative duration to produce already-expired tokens in tests
pub fn mint(
    keys: &SigningKeys,
    issuer: &str,
    sub: &str,
    audience: &str,
    scopes: &[&str],
    ttl: chrono::Duration,
) -> Result<String> {
    let now = Utc::now();
    let iat = now.timestamp() as usize;
    let exp = (now + ttl).timestamp() as usize;

    let claims = TokenClaims {
        iss: issuer.to_owned(),
        sub: sub.to_owned(),
        aud: vec![audience.to_owned()],
        exp,
        iat,
        jti: Uuid::new_v4().to_string(),
        scope: scopes.join(" "),
    };

    let header = Header {
        alg: Algorithm::EdDSA,
        kid: Some(keys.kid.clone()),
        ..Header::new(Algorithm::EdDSA)
    };

    jsonwebtoken::encode(&header, &claims, &keys.encoding).map_err(Into::into)
}

/// Verify a JWT access token and return the decoded claims.
///
/// Validation enforces: EdDSA algorithm, `exp` present and not expired,
/// `aud` matches `expected_aud`, `iss` matches `expected_iss`, leeway = 0 s.
pub fn verify(
    keys: &SigningKeys,
    token: &str,
    expected_aud: &str,
    expected_iss: &str,
) -> Result<TokenClaims> {
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.leeway = 0;
    validation.set_audience(&[expected_aud]);
    validation.set_issuer(&[expected_iss]);
    validation.set_required_spec_claims(&["exp", "iss", "aud"]);

    let data = jsonwebtoken::decode::<TokenClaims>(token, &keys.decoding, &validation)
        .map_err(AuthError::from)?;
    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AuthError;
    use crate::keys::SigningKeys;
    use crate::scope;

    fn keys() -> SigningKeys {
        SigningKeys::generate().expect("generate keys")
    }

    const ISS: &str = "https://auth.example.com";
    const AUD: &str = "https://api.example.com";

    #[test]
    fn mint_verify_roundtrip_preserves_claims() {
        let k = keys();
        let scopes = &[scope::LEDGER_READ, scope::LEDGER_POST];
        let token = mint(
            &k,
            ISS,
            "user-1",
            AUD,
            scopes,
            chrono::Duration::seconds(300),
        )
        .expect("mint");
        let claims = verify(&k, &token, AUD, ISS).expect("verify");
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.aud, vec![AUD.to_owned()]);
        assert_eq!(claims.iss, ISS);
        assert!(claims.has_scope(scope::LEDGER_READ));
        assert!(claims.has_scope(scope::LEDGER_POST));
        assert!(!claims.has_scope(scope::AUDIT_READ));
        let parsed = claims.scopes();
        assert_eq!(parsed.len(), 2);
        assert!(parsed.contains(&scope::LEDGER_READ));
        assert!(parsed.contains(&scope::LEDGER_POST));
    }

    #[test]
    fn tampered_token_returns_token_invalid() {
        let k = keys();
        let token = mint(
            &k,
            ISS,
            "sub",
            AUD,
            &[scope::LEDGER_READ],
            chrono::Duration::seconds(300),
        )
        .expect("mint");
        // Corrupt the last character of the signature segment.
        let mut tampered = token.clone();
        let last = tampered.pop().unwrap();
        let replacement = if last == 'A' { 'B' } else { 'A' };
        tampered.push(replacement);
        let err = verify(&k, &tampered, AUD, ISS).unwrap_err();
        assert!(
            matches!(err, AuthError::TokenInvalid),
            "expected TokenInvalid, got: {err:?}"
        );
    }

    #[test]
    fn expired_token_returns_token_expired() {
        let k = keys();
        let token = mint(
            &k,
            ISS,
            "sub",
            AUD,
            &[scope::LEDGER_READ],
            chrono::Duration::seconds(-10),
        )
        .expect("mint");
        let err = verify(&k, &token, AUD, ISS).unwrap_err();
        assert!(
            matches!(err, AuthError::TokenExpired),
            "expected TokenExpired, got: {err:?}"
        );
    }

    #[test]
    fn wrong_audience_returns_audience_mismatch() {
        let k = keys();
        let token = mint(
            &k,
            ISS,
            "sub",
            AUD,
            &[scope::LEDGER_READ],
            chrono::Duration::seconds(300),
        )
        .expect("mint");
        let err = verify(&k, &token, "https://other.example.com", ISS).unwrap_err();
        assert!(
            matches!(err, AuthError::AudienceMismatch),
            "expected AudienceMismatch, got: {err:?}"
        );
    }

    #[test]
    fn wrong_issuer_returns_issuer_mismatch() {
        let k = keys();
        let token = mint(
            &k,
            ISS,
            "sub",
            AUD,
            &[scope::LEDGER_READ],
            chrono::Duration::seconds(300),
        )
        .expect("mint");
        let err = verify(&k, &token, AUD, "https://evil.example.com").unwrap_err();
        assert!(
            matches!(err, AuthError::IssuerMismatch),
            "expected IssuerMismatch, got: {err:?}"
        );
    }

    #[test]
    fn different_keypairs_produce_different_kids_and_cross_verify_fails() {
        let k1 = keys();
        let k2 = keys();
        assert_ne!(
            k1.kid, k2.kid,
            "two generate() calls must have different kid"
        );

        let token = mint(
            &k1,
            ISS,
            "sub",
            AUD,
            &[scope::LEDGER_READ],
            chrono::Duration::seconds(300),
        )
        .expect("mint with k1");
        let err = verify(&k2, &token, AUD, ISS).unwrap_err();
        assert!(
            matches!(err, AuthError::TokenInvalid),
            "cross-keypair verification should fail with TokenInvalid, got: {err:?}"
        );
    }
}
