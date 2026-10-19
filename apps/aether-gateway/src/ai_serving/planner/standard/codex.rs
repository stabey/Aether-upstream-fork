#[cfg(test)]
#[path = "codex/tests.rs"]
mod tests;

use std::collections::BTreeMap;

use http::HeaderMap;
use serde_json::Value;

pub(crate) use crate::ai_serving::{
    apply_codex_openai_responses_identity_headers, apply_codex_openai_responses_special_body_edits,
    apply_codex_openai_special_headers,
};

/// `client_metadata` key Codex uses to repeat its WebSocket upgrade header on
/// every `response.create`, so the Lite mode survives socket reuse.
const CODEX_RESPONSES_LITE_CLIENT_METADATA_KEY: &str =
    "ws_request_header_x_openai_internal_codex_responses_lite";

fn is_true_flag(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("true")
}

/// Returns whether the downstream client asserted the Responses Lite
/// contract, either with the HTTP/WebSocket-upgrade header or with the
/// per-turn `client_metadata` copy Codex sends over WebSocket.
pub(crate) fn client_requests_codex_responses_lite(
    headers: &HeaderMap,
    client_body: Option<&Value>,
) -> bool {
    headers
        .get(crate::ai_serving::CODEX_RESPONSES_LITE_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_true_flag)
        || client_body
            .and_then(|body| body.get("client_metadata"))
            .and_then(|metadata| metadata.get(CODEX_RESPONSES_LITE_CLIENT_METADATA_KEY))
            .and_then(Value::as_str)
            .is_some_and(is_true_flag)
}

/// Returns whether an already planned provider request carries the Lite
/// header. Later passes over the same decision (routing policy mutations)
/// follow the first pass's client-derived choice instead of re-deriving it.
pub(crate) fn planned_codex_request_uses_responses_lite(
    provider_request_headers: &BTreeMap<String, String>,
) -> bool {
    provider_request_headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case(crate::ai_serving::CODEX_RESPONSES_LITE_HEADER)
            && is_true_flag(value)
    })
}

pub(crate) fn codex_model_capabilities_for_transport(
    transport: &crate::ai_serving::GatewayProviderTransportSnapshot,
    provider_api_format: &str,
    provider_model: &str,
    source_model: &str,
    client_requests_lite: bool,
) -> Option<crate::ai_serving::CodexResponsesModelCapabilities> {
    codex_model_capabilities(
        &transport.provider.provider_type,
        provider_api_format,
        provider_model,
        source_model,
        transport.key.upstream_metadata.as_ref(),
        client_requests_lite,
    )
}

fn codex_model_capabilities(
    provider_type: &str,
    provider_api_format: &str,
    provider_model: &str,
    source_model: &str,
    upstream_metadata: Option<&Value>,
    client_requests_lite: bool,
) -> Option<crate::ai_serving::CodexResponsesModelCapabilities> {
    let uses_codex_model_catalog =
        crate::ai_serving::is_openai_responses_family_format(provider_api_format)
            || crate::ai_serving::api_format_alias_matches(provider_api_format, "openai:search");
    if !provider_type.trim().eq_ignore_ascii_case("codex") || !uses_codex_model_catalog {
        return None;
    }
    Some(
        crate::ai_serving::resolve_codex_responses_model_capabilities_for_request(
            provider_model,
            source_model,
            upstream_metadata,
            client_requests_lite,
        ),
    )
}
