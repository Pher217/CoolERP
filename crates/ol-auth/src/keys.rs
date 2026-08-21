use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use ed25519_dalek::pkcs8::spki::EncodePublicKey;
use getrandom::{SysRng, rand_core::UnwrapErr};
use jsonwebtoken::{DecodingKey, EncodingKey};
use pkcs8::LineEnding;
use sha2::{Digest, Sha256};

use crate::error::{AuthError, Result};

/// Holds a live Ed25519 signing/verifying key pair with pre-built JWT keys.
pub struct SigningKeys {
    pub encoding: EncodingKey,
    pub decoding: DecodingKey,
    /// Stable key identifier: base64url-nopad of SHA-256(public_raw)[..16].
    pub kid: String,
    /// Raw 32-byte Ed25519 public key (used for JWK `x` field).
    pub public_raw: [u8; 32],
}

impl SigningKeys {
    /// Generate a fresh Ed25519 key pair using the OS random source.
    pub fn generate() -> Result<Self> {
        let signing_key = SigningKey::generate(&mut UnwrapErr(SysRng));
        Self::from_dalek_key(&signing_key)
    }

    /// Load keys from PEM-encoded PKCS#8 private key and SPKI public key strings.
    pub fn from_pem(private_pem: &str, public_pem: &str) -> Result<Self> {
        let encoding = EncodingKey::from_ed_pem(private_pem.as_bytes())
            .map_err(|e| AuthError::Key(e.to_string()))?;
        let decoding = DecodingKey::from_ed_pem(public_pem.as_bytes())
            .map_err(|e| AuthError::Key(e.to_string()))?;

        // Decode the public key bytes to compute kid and public_raw.
        // The SPKI PEM for Ed25519 contains the raw 32-byte public key as the
        // BIT STRING payload. We parse with ed25519-dalek to extract the bytes.
        use ed25519_dalek::pkcs8::spki::DecodePublicKey;
        let verifying_key = ed25519_dalek::VerifyingKey::from_public_key_pem(public_pem)
            .map_err(|e| AuthError::Key(e.to_string()))?;
        let public_raw = verifying_key.to_bytes();
        let kid = compute_kid(&public_raw);

        Ok(SigningKeys {
            encoding,
            decoding,
            kid,
            public_raw,
        })
    }

    /// Export private and public PEM strings (PKCS#8 / SPKI).
    /// Useful for persisting a generated key pair to environment variables.
    pub fn export_pems(&self) -> Result<(String, String)> {
        // Re-derive the signing key from the public bytes is not possible — we
        // need to store PEMs at generate() time. Rebuild here via dalek only if
        // we had stored the raw private key. Instead, require callers to use
        // `generate_with_pems()` when they need the PEM strings.
        Err(AuthError::Key(
            "export_pems not available on loaded keys; use generate_with_pems()".into(),
        ))
    }

    /// Generate a fresh key pair and also return the PEM strings so they can
    /// be persisted (e.g. to environment variables).
    pub fn generate_with_pems() -> Result<(Self, String, String)> {
        let signing_key = SigningKey::generate(&mut UnwrapErr(SysRng));
        let private_pem = signing_key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|e| AuthError::Key(e.to_string()))?
            .to_string();
        let public_pem = signing_key
            .verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .map_err(|e| AuthError::Key(e.to_string()))?;
        let keys = Self::from_dalek_key(&signing_key)?;
        Ok((keys, private_pem, public_pem))
    }

    /// Build a JWK Set document for the public key.
    pub fn jwks(&self) -> serde_json::Value {
        let x = URL_SAFE_NO_PAD.encode(self.public_raw);
        serde_json::json!({
            "keys": [{
                "kty": "OKP",
                "crv": "Ed25519",
                "use": "sig",
                "alg": "EdDSA",
                "kid": self.kid,
                "x": x
            }]
        })
    }

    fn from_dalek_key(signing_key: &SigningKey) -> Result<Self> {
        let private_pem = signing_key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|e| AuthError::Key(e.to_string()))?;
        let public_pem = signing_key
            .verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .map_err(|e| AuthError::Key(e.to_string()))?;

        let encoding = EncodingKey::from_ed_pem(private_pem.as_bytes())
            .map_err(|e| AuthError::Key(e.to_string()))?;
        let decoding = DecodingKey::from_ed_pem(public_pem.as_bytes())
            .map_err(|e| AuthError::Key(e.to_string()))?;

        let public_raw = signing_key.verifying_key().to_bytes();
        let kid = compute_kid(&public_raw);

        Ok(SigningKeys {
            encoding,
            decoding,
            kid,
            public_raw,
        })
    }
}

fn compute_kid(public_raw: &[u8; 32]) -> String {
    let hash = Sha256::digest(public_raw);
    URL_SAFE_NO_PAD.encode(&hash[..16])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope;
    use crate::token::{mint, verify};

    const ISS: &str = "https://auth.example.com";
    const AUD: &str = "https://api.example.com";

    #[test]
    fn from_pem_roundtrip_mint_verify() {
        let (keys, private_pem, public_pem) =
            SigningKeys::generate_with_pems().expect("generate_with_pems");
        let loaded = SigningKeys::from_pem(&private_pem, &public_pem).expect("from_pem");
        assert_eq!(keys.kid, loaded.kid);
        assert_eq!(keys.public_raw, loaded.public_raw);

        let token = mint(
            &loaded,
            ISS,
            "user-pem",
            AUD,
            &[scope::AUDIT_READ],
            chrono::Duration::seconds(60),
        )
        .expect("mint");
        let claims = verify(&loaded, &token, AUD, ISS).expect("verify");
        assert_eq!(claims.sub, "user-pem");
    }

    #[test]
    fn jwks_fields() {
        let keys = SigningKeys::generate().expect("generate");
        let jwks = keys.jwks();
        let key = &jwks["keys"][0];
        assert_eq!(key["kty"], "OKP");
        assert_eq!(key["crv"], "Ed25519");
        assert_eq!(key["use"], "sig");
        assert_eq!(key["alg"], "EdDSA");
        assert_eq!(key["kid"], keys.kid.as_str());
        // x is non-empty base64url
        let x = key["x"].as_str().expect("x must be a string");
        assert!(!x.is_empty());
        // Confirm x decodes to 32 bytes (Ed25519 public key)
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(x)
            .expect("x is valid base64url");
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn two_generates_have_different_kids() {
        let k1 = SigningKeys::generate().expect("k1");
        let k2 = SigningKeys::generate().expect("k2");
        assert_ne!(k1.kid, k2.kid);
    }
}
