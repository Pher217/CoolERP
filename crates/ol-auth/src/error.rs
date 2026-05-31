use jsonwebtoken::errors::ErrorKind;
use thiserror::Error;

/// Errors produced by the ol-auth crypto/token core.
#[derive(Debug, Error)]
pub enum AuthError {
    /// Key generation or loading failure.
    #[error("key error: {0}")]
    Key(String),

    /// JWT signature or format is invalid (catch-all for non-specific JWT errors).
    #[error("token invalid")]
    TokenInvalid,

    /// JWT `exp` claim is in the past.
    #[error("token expired")]
    TokenExpired,

    /// JWT `aud` claim does not match the expected audience.
    #[error("audience mismatch")]
    AudienceMismatch,

    /// JWT `iss` claim does not match the expected issuer.
    #[error("issuer mismatch")]
    IssuerMismatch,

    /// Password hashing or verification error.
    #[error("password error: {0}")]
    Password(String),
}

impl From<jsonwebtoken::errors::Error> for AuthError {
    fn from(e: jsonwebtoken::errors::Error) -> Self {
        match e.kind() {
            ErrorKind::ExpiredSignature => AuthError::TokenExpired,
            ErrorKind::InvalidAudience => AuthError::AudienceMismatch,
            ErrorKind::InvalidIssuer => AuthError::IssuerMismatch,
            _ => AuthError::TokenInvalid,
        }
    }
}

pub type Result<T> = std::result::Result<T, AuthError>;
