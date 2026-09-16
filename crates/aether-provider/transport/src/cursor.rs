//! Cursor provider transport helpers.
//!
//! Codex is the mature in-Aether pattern: the gateway talks HTTP in-process
//! because ChatGPT Codex already exposes an HTTP Responses API. Cursor's
//! official Agent harness is `@cursor/sdk`, which is Node — it cannot be
//! linked into the distroless `aether-gateway` binary (no Node runtime, and
//! compose also runs the app container `read_only` with `noexec` `/tmp`).
//!
//! Same-image interaction therefore follows `aether-vscodex`: a Node sidecar
//! in the Compose image talks `@cursor/sdk`, and this Rust process calls it
//! over the internal URL (`AETHER_CURSOR_SDK_INTERNAL_URL`). Operators can
//! still point at an external cursor-sdk2api / Cursor2API gateway.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::snapshot::GatewayProviderTransportSnapshot;

pub const CURSOR_PROVIDER_TYPE: &str = "cursor";

/// Loopback OpenAI-compatible base URL for a locally spawned SDK sidecar.
/// Override with `AETHER_CURSOR_SDK_INTERNAL_URL` in Docker (for example
/// `http://cursor-sdk:8792/v1`).
pub const CURSOR_DEFAULT_GATEWAY_BASE_URL: &str = "http://127.0.0.1:8792/v1";
pub const CURSOR_INTERNAL_URL_ENV: &str = "AETHER_CURSOR_SDK_INTERNAL_URL";
pub const CURSOR_SDK_CLIENT_VERSION: &str = "1.0.30";
pub const CURSOR_CLIENT_TYPE_HEADER: &str = "x-cursor-client-type";
pub const CURSOR_CLIENT_VERSION_HEADER: &str = "x-cursor-client-version";
pub const CURSOR_CLIENT_TYPE_VALUE: &str = "sdk";

const LEGACY_LOOPBACK_BASE_URLS: &[&str] = &[
    CURSOR_DEFAULT_GATEWAY_BASE_URL,
    "http://127.0.0.1:8080/v1",
    "http://localhost:8792/v1",
    "http://localhost:8080/v1",
];

pub fn is_cursor_provider_type(provider_type: &str) -> bool {
    provider_type
        .trim()
        .eq_ignore_ascii_case(CURSOR_PROVIDER_TYPE)
}

pub fn is_cursor_provider_transport(transport: &GatewayProviderTransportSnapshot) -> bool {
    is_cursor_provider_type(&transport.provider.provider_type)
}

pub fn cursor_sdk_internal_base_url() -> String {
    static CACHED: OnceLock<String> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            std::env::var(CURSOR_INTERNAL_URL_ENV)
                .ok()
                .map(|value| trim_base_url(&value))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| CURSOR_DEFAULT_GATEWAY_BASE_URL.to_string())
        })
        .clone()
}

pub fn resolved_cursor_upstream_base_url(
    transport: &GatewayProviderTransportSnapshot,
) -> Option<String> {
    if !is_cursor_provider_transport(transport) {
        return None;
    }
    Some(resolve_cursor_base_url(
        &transport.endpoint.base_url,
        cursor_sdk_internal_base_url(),
    ))
}

pub fn resolved_cursor_request_base_url(transport: &GatewayProviderTransportSnapshot) -> String {
    resolved_cursor_upstream_base_url(transport)
        .unwrap_or_else(|| trim_base_url(&transport.endpoint.base_url))
}

pub fn insert_cursor_sdk_identity_headers(headers: &mut BTreeMap<String, String>) {
    headers
        .entry(CURSOR_CLIENT_TYPE_HEADER.to_string())
        .or_insert_with(|| CURSOR_CLIENT_TYPE_VALUE.to_string());
    headers
        .entry(CURSOR_CLIENT_VERSION_HEADER.to_string())
        .or_insert_with(|| CURSOR_SDK_CLIENT_VERSION.to_string());
    headers
        .entry("user-agent".to_string())
        .or_insert_with(|| format!("@cursor/sdk/{CURSOR_SDK_CLIENT_VERSION}"));
}

pub fn insert_cursor_sdk_identity_headers_if_needed(
    transport: &GatewayProviderTransportSnapshot,
    headers: &mut BTreeMap<String, String>,
) {
    if is_cursor_provider_transport(transport) {
        insert_cursor_sdk_identity_headers(headers);
    }
}

fn resolve_cursor_base_url(stored: &str, internal: String) -> String {
    if is_default_or_empty_cursor_base(stored) {
        internal
    } else {
        trim_base_url(stored)
    }
}

fn is_default_or_empty_cursor_base(stored: &str) -> bool {
    let trimmed = trim_base_url(stored);
    trimmed.is_empty()
        || LEGACY_LOOPBACK_BASE_URLS
            .iter()
            .any(|candidate| trimmed.eq_ignore_ascii_case(candidate))
}

fn trim_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        GatewayProviderTransportEndpoint, GatewayProviderTransportKey,
        GatewayProviderTransportProvider, GatewayProviderTransportSnapshot,
    };

    fn sample_transport(provider_type: &str, base_url: &str) -> GatewayProviderTransportSnapshot {
        GatewayProviderTransportSnapshot {
            provider: GatewayProviderTransportProvider {
                id: "provider-cursor".to_string(),
                name: "Cursor".to_string(),
                provider_type: provider_type.to_string(),
                website: None,
                is_active: true,
                keep_priority_on_conversion: false,
                enable_format_conversion: true,
                concurrent_limit: None,
                max_retries: None,
                proxy: None,
                request_timeout_secs: None,
                stream_first_byte_timeout_secs: None,
                config: None,
            },
            endpoint: GatewayProviderTransportEndpoint {
                id: "endpoint-cursor".to_string(),
                provider_id: "provider-cursor".to_string(),
                api_format: "openai:chat".to_string(),
                api_family: None,
                endpoint_kind: None,
                is_active: true,
                base_url: base_url.to_string(),
                header_rules: None,
                body_rules: None,
                max_retries: None,
                custom_path: None,
                config: None,
                format_acceptance_config: None,
                proxy: None,
            },
            key: GatewayProviderTransportKey {
                id: "key-cursor".to_string(),
                provider_id: "provider-cursor".to_string(),
                name: "key".to_string(),
                auth_type: "bearer".to_string(),
                is_active: true,
                api_formats: None,
                auth_type_by_format: None,
                allow_auth_channel_mismatch_formats: None,
                allowed_models: None,
                capabilities: None,
                rate_multipliers: None,
                global_priority_by_format: None,
                expires_at_unix_secs: None,
                proxy: None,
                fingerprint: None,
                upstream_metadata: None,
                decrypted_api_key: "crsr_test".to_string(),
                decrypted_auth_config: None,
            },
        }
    }

    #[test]
    fn detects_cursor_provider_type() {
        assert!(is_cursor_provider_type("cursor"));
        assert!(is_cursor_provider_type(" Cursor "));
        assert!(!is_cursor_provider_type("custom"));
        assert!(!is_cursor_provider_type("codex"));
        assert_eq!(CURSOR_DEFAULT_GATEWAY_BASE_URL, "http://127.0.0.1:8792/v1");
    }

    #[test]
    fn rewrites_loopback_template_to_internal_sdk_url() {
        let transport = sample_transport("cursor", CURSOR_DEFAULT_GATEWAY_BASE_URL);
        assert_eq!(
            resolved_cursor_upstream_base_url(&transport).as_deref(),
            Some(cursor_sdk_internal_base_url().as_str())
        );
        let legacy = sample_transport("cursor", "http://127.0.0.1:8080/v1");
        assert_eq!(
            resolved_cursor_upstream_base_url(&legacy).as_deref(),
            Some(cursor_sdk_internal_base_url().as_str())
        );
    }

    #[test]
    fn preserves_explicit_custom_cursor_gateway() {
        let transport = sample_transport("cursor", "http://cursor-sdk2api:8080/v1");
        assert_eq!(
            resolved_cursor_upstream_base_url(&transport).as_deref(),
            Some("http://cursor-sdk2api:8080/v1")
        );
        assert!(resolved_cursor_upstream_base_url(&sample_transport("codex", "")).is_none());
    }

    #[test]
    fn attaches_sdk_identity_headers() {
        let transport = sample_transport("cursor", CURSOR_DEFAULT_GATEWAY_BASE_URL);
        let mut headers = BTreeMap::new();
        insert_cursor_sdk_identity_headers_if_needed(&transport, &mut headers);
        assert_eq!(
            headers.get(CURSOR_CLIENT_TYPE_HEADER).map(String::as_str),
            Some(CURSOR_CLIENT_TYPE_VALUE)
        );
        assert_eq!(
            headers
                .get(CURSOR_CLIENT_VERSION_HEADER)
                .map(String::as_str),
            Some(CURSOR_SDK_CLIENT_VERSION)
        );

        let mut other = BTreeMap::new();
        insert_cursor_sdk_identity_headers_if_needed(&sample_transport("codex", ""), &mut other);
        assert!(other.is_empty());
    }

    #[test]
    fn resolve_helper_keeps_custom_and_rewrites_empty() {
        assert_eq!(
            resolve_cursor_base_url("", "http://cursor-sdk:8792/v1".to_string()),
            "http://cursor-sdk:8792/v1"
        );
        assert_eq!(
            resolve_cursor_base_url(
                "http://127.0.0.1:8792/v1/",
                "http://cursor-sdk:8792/v1".to_string()
            ),
            "http://cursor-sdk:8792/v1"
        );
        assert_eq!(
            resolve_cursor_base_url(
                "https://gateway.example/v1",
                "http://cursor-sdk:8792/v1".to_string()
            ),
            "https://gateway.example/v1"
        );
    }
}
