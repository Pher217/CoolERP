//! `ol-auth` — pure crypto/token core for the CoolERP OAuth 2.1 Authorization Server.
//!
//! # Security notice
//!
//! This crate is **security-critical**. It handles key generation, JWT minting/verification,
//! and password hashing. HTTP transport, database persistence, and endpoint wiring live in
//! separate crates and are explicitly excluded from this one.
//!
//! Algorithms:
//! - Tokens: EdDSA over Ed25519 via [`jsonwebtoken`] + [`ed25519_dalek`].
//! - Passwords: Argon2id via [`argon2`].
//! - Key fingerprints: SHA-256 via [`sha2`].

pub mod error;
pub mod keys;
pub mod password;
pub mod scope;
pub mod token;

pub use error::{AuthError, Result};
pub use keys::SigningKeys;
pub use token::{TokenClaims, mint, verify};
