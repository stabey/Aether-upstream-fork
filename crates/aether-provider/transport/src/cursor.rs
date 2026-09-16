//! Cursor provider transport helpers.
//!
//! Cursor's official Agent harness lives in `@cursor/sdk` (Node). Aether does
//! not embed that SDK. Instead the `cursor` fixed provider talks OpenAI /
//! Anthropic-compatible HTTP to an external SDK gateway such as
//! [cursor-sdk2api](https://github.com/Sunnyender-org/cursor-sdk2api) or
//! [Cursor2API](https://github.com/NGLSG/Cursor2API).
//!
//! Operators store Cursor User API Keys (`crsr_…`) as Aether provider keys and
//! run the sidecar in BYOK mode so Aether can pool and schedule those keys.

use crate::snapshot::GatewayProviderTransportSnapshot;

pub const CURSOR_PROVIDER_TYPE: &str = "cursor";

/// Default OpenAI/Anthropic-compatible base URL for a local cursor-sdk2api /
/// Cursor2API sidecar. Override for Docker networks (for example
/// `http://cursor-sdk2api:8080/v1`).
pub const CURSOR_DEFAULT_GATEWAY_BASE_URL: &str = "http://127.0.0.1:8080/v1";

pub fn is_cursor_provider_type(provider_type: &str) -> bool {
    provider_type
        .trim()
        .eq_ignore_ascii_case(CURSOR_PROVIDER_TYPE)
}

pub fn is_cursor_provider_transport(transport: &GatewayProviderTransportSnapshot) -> bool {
    is_cursor_provider_type(&transport.provider.provider_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cursor_provider_type() {
        assert!(is_cursor_provider_type("cursor"));
        assert!(is_cursor_provider_type(" Cursor "));
        assert!(!is_cursor_provider_type("custom"));
        assert!(!is_cursor_provider_type("xai"));
        assert_eq!(CURSOR_DEFAULT_GATEWAY_BASE_URL, "http://127.0.0.1:8080/v1");
    }
}
