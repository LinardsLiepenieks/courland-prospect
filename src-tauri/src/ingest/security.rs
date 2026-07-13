//! Pure request-authorization checks for the loopback ingest server. Kept free
//! of axum/Tauri types so the security-critical logic is unit-tested in
//! isolation. Three independent defenses, each closing a different hole:
//!
//!  - `host_ok`   — rejects DNS-rebinding (a malicious site rebinding its domain
//!                  to 127.0.0.1 sends its own host in the `Host` header).
//!  - `token_ok`  — a shared secret only our extension knows (constant-time).
//!  - `origin_ok` — locks CORS/writes to exactly our extension's origin.

/// Header the extension sends the shared token in.
pub const TOKEN_HEADER: &str = "x-courland-token";

/// The `Host` header must name loopback on our exact port — nothing else.
/// A rebound request carries the attacker's domain here, so this rejects it.
pub fn host_ok(host: Option<&str>, app_port: u16) -> bool {
    match host {
        Some(h) => h == format!("127.0.0.1:{app_port}") || h == format!("localhost:{app_port}"),
        None => false,
    }
}

/// The token must be present and equal to the expected value. Constant-time to
/// avoid leaking a match prefix via timing.
pub fn token_ok(provided: Option<&str>, expected: &str) -> bool {
    match provided {
        Some(t) => constant_time_eq(t.as_bytes(), expected.as_bytes()),
        None => false,
    }
}

/// The `Origin` (when present) must be exactly our extension's origin.
pub fn origin_ok(origin: Option<&str>, extension_id: &str) -> bool {
    match origin {
        Some(o) => o == format!("chrome-extension://{extension_id}"),
        None => false,
    }
}

/// Length-aware constant-time byte comparison. Returns false for differing
/// lengths without an early-return that would leak length via timing beyond the
/// unavoidable length check itself.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_accepts_loopback_on_exact_port() {
        assert!(host_ok(Some("127.0.0.1:47823"), 47823));
        assert!(host_ok(Some("localhost:47823"), 47823));
    }

    #[test]
    fn host_rejects_wrong_port_missing_and_rebinding() {
        assert!(!host_ok(Some("127.0.0.1:1234"), 47823));
        assert!(!host_ok(Some("evil.example.com:47823"), 47823)); // DNS rebind
        assert!(!host_ok(Some("127.0.0.1"), 47823)); // no port
        assert!(!host_ok(None, 47823));
    }

    #[test]
    fn token_matches_only_exact_value() {
        assert!(token_ok(Some("s3cret"), "s3cret"));
        assert!(!token_ok(Some("s3cre"), "s3cret")); // prefix
        assert!(!token_ok(Some("s3crett"), "s3cret")); // longer
        assert!(!token_ok(Some("wrong"), "s3cret"));
        assert!(!token_ok(None, "s3cret"));
    }

    #[test]
    fn origin_matches_only_our_extension() {
        let id = "knipphmpmemfkimdiknnjjbelecnkenf";
        assert!(origin_ok(Some(&format!("chrome-extension://{id}")), id));
        assert!(!origin_ok(Some("chrome-extension://someotherid"), id));
        assert!(!origin_ok(Some("https://www.linkedin.com"), id));
        assert!(!origin_ok(None, id));
    }
}
