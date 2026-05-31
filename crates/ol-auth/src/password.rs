use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand::rngs::OsRng;

use crate::error::{AuthError, Result};

/// Hash a plaintext password with Argon2id using a freshly generated salt.
///
/// Returns a PHC-format string (e.g. `$argon2id$v=19$...`) suitable for
/// long-term storage. Each call produces a unique salt.
pub fn hash_password(pw: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map_err(|e| AuthError::Password(e.to_string()))?;
    Ok(hash.to_string())
}

/// Verify `candidate` against a stored PHC hash string.
///
/// Returns `Ok(true)` on a match, `Ok(false)` on a mismatch, and
/// `Err(AuthError::Password)` only when `phc` is malformed.
pub fn verify_password(phc: &str, candidate: &str) -> Result<bool> {
    let parsed = PasswordHash::new(phc).map_err(|e| AuthError::Password(e.to_string()))?;
    match Argon2::default().verify_password(candidate.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(AuthError::Password(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AuthError;

    #[test]
    fn hash_starts_with_argon2id() {
        let hash = hash_password("correct-horse-battery-staple").expect("hash");
        assert!(
            hash.starts_with("$argon2id$"),
            "expected PHC prefix $argon2id$, got: {hash}"
        );
    }

    #[test]
    fn verify_correct_password_returns_true() {
        let pw = "super-secret-password";
        let hash = hash_password(pw).expect("hash");
        assert!(verify_password(&hash, pw).expect("verify"));
    }

    #[test]
    fn verify_wrong_password_returns_false() {
        let hash = hash_password("correct-password").expect("hash");
        assert!(!verify_password(&hash, "wrong-password").expect("verify"));
    }

    #[test]
    fn malformed_phc_returns_password_error() {
        let err = verify_password("not-a-valid-phc-string", "pw").unwrap_err();
        assert!(
            matches!(err, AuthError::Password(_)),
            "expected Password error, got: {err:?}"
        );
    }
}
