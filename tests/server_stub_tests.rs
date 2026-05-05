//! Pure-function tests for `server::is_warp_stub_path`, the gate that
//! decides which non-`/ai/multi-agent` paths the Slice-4 warp-stub layer
//! answers with a 200. Kept tiny and pure — a full end-to-end HTTPS test
//! would require cert fixtures + a running server and is covered by the
//! existing `server_boot_test.rs` integration test.

use warp_byok_proxy::server::is_warp_stub_path;

#[test]
fn stubs_graphql_endpoint() {
    assert!(is_warp_stub_path("/graphql"));
}

#[test]
fn stubs_every_subpath_under_auth() {
    assert!(is_warp_stub_path("/auth/"));
    assert!(is_warp_stub_path("/auth/anonymous"));
    assert!(is_warp_stub_path("/auth/refresh-token"));
    assert!(is_warp_stub_path("/auth/whoever"));
}

#[test]
fn does_not_stub_ai_or_health_or_unknown_paths() {
    // The stub MUST NOT swallow the real endpoints or unknown paths — those
    // continue to route through the normal match above.
    assert!(!is_warp_stub_path("/ai/multi-agent"));
    assert!(!is_warp_stub_path("/health"));
    assert!(!is_warp_stub_path("/"));
    assert!(!is_warp_stub_path("/graphql/extra"));
    assert!(!is_warp_stub_path("/authz"));
    assert!(!is_warp_stub_path("/auth")); // exact prefix only, trailing slash required
}
