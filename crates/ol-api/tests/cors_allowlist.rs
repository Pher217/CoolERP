//! CORS must be an allowlist, not permissive.
//!
//! Today `app` builds the router with `tower_http::cors::CorsLayer::permissive()`,
//! which echoes an `access-control-allow-origin` header for every origin. This
//! test pins the security posture we want instead: only the configured local dev
//! origin (the Vite SPA at `http://localhost:5173`) is allowed, and an unknown
//! origin must receive NO `access-control-allow-origin` header at all.
//!
//! The test is expected to FAIL until `app` is tightened to an explicit allowlist.

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use ol_api::app;
use sqlx::PgPool;
use tower::ServiceExt;

/// The local dev origin the SPA is served from (Vite default).
const LOCAL_DEV_ORIGIN: &str = "http://localhost:5173";

/// An origin that must never be allowed.
const EVIL_ORIGIN: &str = "https://evil.example";

/// Send `GET /health` with the given `Origin` and return the value of the
/// `access-control-allow-origin` response header, if present.
async fn allow_origin_for(pool: PgPool, origin: &str) -> Option<String> {
    let router = app(pool);
    let req = Request::builder()
        .method("GET")
        .uri("/health")
        .header(header::ORIGIN, origin)
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    resp.headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .map(|v| v.to_str().unwrap().to_owned())
}

#[sqlx::test(migrations = "../../migrations")]
async fn cors_is_an_allowlist_not_permissive(pool: PgPool) {
    // GIVEN the API ships with a CORS allowlist scoped to the local dev origin.

    // WHEN a request arrives from an unknown, untrusted origin,
    // THEN it must NOT receive an access-control-allow-origin header.
    let denied = allow_origin_for(pool.clone(), EVIL_ORIGIN).await;
    assert!(
        denied.is_none(),
        "untrusted origin {EVIL_ORIGIN} must not receive an access-control-allow-origin \
         header, got: {denied:?}"
    );

    // WHEN a request arrives from the configured local dev origin,
    // THEN it is allowed and receives an access-control-allow-origin header
    // echoing that origin (an allowlist echoes the specific origin; permissive
    // would emit a wildcard `*`).
    let allowed = allow_origin_for(pool, LOCAL_DEV_ORIGIN).await;
    assert_eq!(
        allowed.as_deref(),
        Some(LOCAL_DEV_ORIGIN),
        "the configured local dev origin must be allowed"
    );
}
