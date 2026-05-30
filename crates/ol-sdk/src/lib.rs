//! ol-sdk — typed client SDK for the OpenLedger MCP / REST API.
//!
//! Licensed MIT OR Apache-2.0 (permissive, frictionless integration; ADR-003).
//! Stage 1: typed request/response structs mirroring the capability surface in
//! `api-surface.md`, with the shared structured-error envelope.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Top-level error envelope returned by the MCP/REST API on failure.
///
/// Wire format: `{ "error": { "code": "...", "message": "...", ... } }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub error: ApiError,
}

impl ErrorEnvelope {
    /// Wrap an [`ApiError`] in the envelope.
    pub fn new(error: ApiError) -> Self {
        Self { error }
    }
}

/// Structured API error matching the OpenLedger error contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Error)]
#[error("{code}: {message}")]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ApiError {
    /// Minimal constructor.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            entity_id: None,
            details: None,
        }
    }

    /// Attach an entity ID (builder style).
    pub fn with_entity_id(mut self, entity_id: impl Into<String>) -> Self {
        self.entity_id = Some(entity_id.into());
        self
    }

    /// Attach arbitrary JSON details (builder style).
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Closed enum of all API error codes, serialized as SCREAMING_SNAKE_CASE strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    UnbalancedEntry,
    DuplicateIdempotencyKey,
    AccountNotFound,
    JournalNotFound,
    InsufficientScope,
    PeriodClosed,
    NegativeStock,
    Validation,
    /// Mutation rejected because the target resource is append-only.
    AppendOnly,
    /// Serialization failure after all retry attempts were exhausted.
    SerializationFailure,
    /// Unexpected internal / database error.
    Internal,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ErrorCode::UnbalancedEntry => "UNBALANCED_ENTRY",
            ErrorCode::DuplicateIdempotencyKey => "DUPLICATE_IDEMPOTENCY_KEY",
            ErrorCode::AccountNotFound => "ACCOUNT_NOT_FOUND",
            ErrorCode::JournalNotFound => "JOURNAL_NOT_FOUND",
            ErrorCode::InsufficientScope => "INSUFFICIENT_SCOPE",
            ErrorCode::PeriodClosed => "PERIOD_CLOSED",
            ErrorCode::NegativeStock => "NEGATIVE_STOCK",
            ErrorCode::Validation => "VALIDATION",
            ErrorCode::AppendOnly => "APPEND_ONLY",
            ErrorCode::SerializationFailure => "SERIALIZATION_FAILURE",
            ErrorCode::Internal => "INTERNAL",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Round-trip with all optional fields set.
    #[test]
    fn round_trip_full() {
        let original = ErrorEnvelope::new(
            ApiError::new(ErrorCode::AccountNotFound, "account not found")
                .with_entity_id("acc_42")
                .with_details(json!({ "account_code": "1000" })),
        );

        let json_str = serde_json::to_string(&original).expect("serialize");
        let deserialized: ErrorEnvelope = serde_json::from_str(&json_str).expect("deserialize");

        assert_eq!(original, deserialized);
        assert_eq!(deserialized.error.entity_id.as_deref(), Some("acc_42"));
        assert!(deserialized.error.details.is_some());
    }

    /// Round-trip with no optional fields (they must be absent from JSON, not null).
    #[test]
    fn round_trip_minimal() {
        let original = ErrorEnvelope::new(ApiError::new(
            ErrorCode::UnbalancedEntry,
            "debits ≠ credits",
        ));

        let json_str = serde_json::to_string(&original).expect("serialize");
        let deserialized: ErrorEnvelope = serde_json::from_str(&json_str).expect("deserialize");

        assert_eq!(original, deserialized);
        assert!(deserialized.error.entity_id.is_none());
        assert!(deserialized.error.details.is_none());

        // Optional fields must be completely absent from the JSON.
        assert!(!json_str.contains("entity_id"), "entity_id must be absent");
        assert!(!json_str.contains("details"), "details must be absent");
    }

    /// The `code` field must serialize as SCREAMING_SNAKE_CASE.
    #[test]
    fn code_serializes_as_screaming_snake() {
        let pairs = [
            (ErrorCode::UnbalancedEntry, "UNBALANCED_ENTRY"),
            (
                ErrorCode::DuplicateIdempotencyKey,
                "DUPLICATE_IDEMPOTENCY_KEY",
            ),
            (ErrorCode::AccountNotFound, "ACCOUNT_NOT_FOUND"),
            (ErrorCode::JournalNotFound, "JOURNAL_NOT_FOUND"),
            (ErrorCode::InsufficientScope, "INSUFFICIENT_SCOPE"),
            (ErrorCode::PeriodClosed, "PERIOD_CLOSED"),
            (ErrorCode::NegativeStock, "NEGATIVE_STOCK"),
            (ErrorCode::Validation, "VALIDATION"),
            (ErrorCode::AppendOnly, "APPEND_ONLY"),
            (ErrorCode::SerializationFailure, "SERIALIZATION_FAILURE"),
            (ErrorCode::Internal, "INTERNAL"),
        ];

        for (code, expected) in pairs {
            let json_str = serde_json::to_string(&code).expect("serialize");
            assert_eq!(json_str, format!("\"{expected}\""), "code={code:?}");

            let deserialized: ErrorCode = serde_json::from_str(&json_str).expect("deserialize");
            assert_eq!(deserialized, code);
        }
    }

    /// Display format is `CODE: message`.
    #[test]
    fn display_format() {
        let err = ApiError::new(ErrorCode::PeriodClosed, "fiscal year 2024 is closed");
        assert_eq!(err.to_string(), "PERIOD_CLOSED: fiscal year 2024 is closed");
    }

    /// ErrorEnvelope wire format matches the spec shape exactly.
    #[test]
    fn wire_shape() {
        let env = ErrorEnvelope::new(
            ApiError::new(ErrorCode::Validation, "invalid amount")
                .with_entity_id("inv_88")
                .with_details(json!({})),
        );

        let v: serde_json::Value = serde_json::to_value(&env).expect("to_value");
        assert_eq!(v["error"]["code"], "VALIDATION");
        assert_eq!(v["error"]["message"], "invalid amount");
        assert_eq!(v["error"]["entity_id"], "inv_88");
        assert!(v["error"]["details"].is_object());
    }

    /// ApiError implements std::error::Error (compile-time check via trait bound).
    #[test]
    fn implements_std_error() {
        fn accepts_error(_: &dyn std::error::Error) {}
        let err = ApiError::new(ErrorCode::InsufficientScope, "missing write:journal");
        accepts_error(&err);
    }
}
