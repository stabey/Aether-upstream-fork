use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use aether_data_contracts::repository::provider_catalog::{
    StoredProviderCatalogEndpoint, StoredProviderCatalogKey,
};
use aether_provider_transport::xai::{
    extract_xai_user_id_from_auth_config, is_xai_provider_transport, xai_auth_uses_api,
};
use aether_provider_transport::GatewayProviderTransportSnapshot;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::strategy::build_success_outcome;
use crate::transport::xai_models_fetch_url;
use crate::{
    aggregate_models_for_cache, build_standard_models_fetch_execution_plan,
    preset_models_for_provider, ModelFetchTransportRuntime, ModelsFetchOutcome,
};

const CATALOG_NAMESPACE: &str = "xai_model_catalog";
const CATALOG_VERSION: u64 = 1;
const FRESH_FOR_SECS: u64 = 6 * 60 * 60;
const MAX_MODELS: usize = 1024;
const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XaiModelCatalogSource {
    Upstream,
    LastSuccessCache,
    StaticFallback,
}

/// Media-only xAI accounts expose the same directory as text accounts.
pub fn selected_xai_models_fetch_endpoints(
    endpoints: &[StoredProviderCatalogEndpoint],
    key: &StoredProviderCatalogKey,
) -> Vec<StoredProviderCatalogEndpoint> {
    let standard = crate::selected_models_fetch_endpoints(endpoints, key);
    if !standard.is_empty() {
        return standard;
    }
    let formats = crate::json_string_list(key.api_formats.as_ref());
    endpoints
        .iter()
        .filter(|endpoint| {
            endpoint.is_active
                && matches!(
                    endpoint.api_format.as_str(),
                    "openai:image" | "openai:video"
                )
                && (formats.is_empty() || formats.contains(&endpoint.api_format))
        })
        .cloned()
        .collect()
}

/// Refreshes each selected xAI account/entry point independently. A failed refresh
/// can return its last successful directory but never publishes a new success or
/// replaces that directory with presets. Persistence uses a separate key metadata
/// namespace, so billing/quota snapshots remain independent.
pub async fn fetch_xai_models_from_transports(
    runtime: &(impl ModelFetchTransportRuntime + ?Sized),
    transports: &[GatewayProviderTransportSnapshot],
    force_refresh: bool,
) -> Result<ModelsFetchOutcome, String> {
    if transports.is_empty() || transports.iter().any(|t| !is_xai_provider_transport(t)) {
        return Err("xAI models fetch requires xAI transport snapshots".to_string());
    }
    // Never merge different accounts into one account's persisted namespace.
    let first = &transports[0];
    if transports
        .iter()
        .any(|t| t.provider.id != first.provider.id || t.key.id != first.key.id)
    {
        return Err("xAI models fetch requires one provider account".to_string());
    }
    let now = current_unix_secs();
    let mut seen = BTreeSet::new();
    let mut catalogs = Map::new();
    let mut models = Vec::new();
    let mut errors = Vec::new();
    let mut live_success = false;
    let mut cached = false;
    let mut upstream_status = None;
    for transport in transports {
        let scope = match catalog_scope(transport) {
            Ok(scope) => scope,
            Err(_) => {
                errors.push("xAI models fetch URL configuration is invalid".to_string());
                continue;
            }
        };
        if !seen.insert(scope.clone()) {
            continue;
        }
        let last = cached_catalog(transport, &scope, now);
        if !force_refresh {
            if let Some((at, cards)) = last
                .as_ref()
                .filter(|(at, _)| now.saturating_sub(*at) < FRESH_FOR_SECS)
            {
                models.extend(cards.iter().cloned());
                catalogs.insert(scope, catalog_entry(*at, cards));
                cached = true;
                continue;
            }
        }
        let attempt = async {
            let plan = build_standard_models_fetch_execution_plan(runtime, transport, None)
                .await
                .map_err(|_| {
                    "xAI models fetch credentials or transport configuration unavailable"
                        .to_string()
                })?;
            let result = runtime
                .execute_model_fetch_execution_plan(&plan)
                .await
                .map_err(|_| "xAI models fetch transport failed".to_string())?;
            upstream_status = Some(result.status_code);
            if result.status_code != 200 {
                // Error bodies/HTTP client diagnostics can reflect credentials.
                return Err(format!(
                    "xAI models fetch failed (HTTP {})",
                    result.status_code
                ));
            }
            let body = result
                .body
                .as_ref()
                .and_then(|b| b.json_body.as_ref())
                .ok_or_else(|| "xAI models response is missing JSON payload".to_string())?;
            parse_catalog(body)
        }
        .await;
        match attempt {
            Ok(cards) => {
                catalogs.insert(scope, catalog_entry(current_unix_secs(), &cards));
                models.extend(cards);
                live_success = true;
            }
            Err(error) => {
                errors.push(error);
                if let Some((at, cards)) = last {
                    catalogs.insert(scope, catalog_entry(at, &cards));
                    models.extend(cards);
                    cached = true;
                }
            }
        }
    }
    let source = if live_success {
        XaiModelCatalogSource::Upstream
    } else if cached {
        XaiModelCatalogSource::LastSuccessCache
    } else {
        models = preset_models_for_provider("xai").unwrap_or_default();
        XaiModelCatalogSource::StaticFallback
    };
    let metadata = live_success.then(|| {
        json!({
            CATALOG_NAMESPACE: {"version": CATALOG_VERSION, "catalogs": catalogs}
        })
    });
    let mut outcome =
        build_success_outcome(aggregate_models_for_cache(&models), metadata, live_success);
    outcome.errors = errors;
    outcome.upstream_status = upstream_status;
    outcome.xai_source = Some(source);
    Ok(outcome)
}

fn catalog_entry(at: u64, cards: &[Value]) -> Value {
    json!({"fetched_at": at, "models": cards})
}

fn cached_catalog(
    transport: &GatewayProviderTransportSnapshot,
    scope: &str,
    now: u64,
) -> Option<(u64, Vec<Value>)> {
    let namespace = transport
        .key
        .upstream_metadata
        .as_ref()?
        .get(CATALOG_NAMESPACE)?;
    if namespace.get("version")?.as_u64()? != CATALOG_VERSION {
        return None;
    }
    let entry = namespace.get("catalogs")?.get(scope)?;
    let at = entry.get("fetched_at")?.as_u64()?;
    if at == 0 || at > now.saturating_add(300) {
        return None;
    }
    Some((at, parse_catalog(entry.get("models")?).ok()?))
}

fn catalog_scope(transport: &GatewayProviderTransportSnapshot) -> Result<String, String> {
    let url = xai_models_fetch_url(transport)?;
    let api = xai_auth_uses_api(
        &transport.key.auth_type,
        transport.key.decrypted_auth_config.as_deref(),
    );
    // Account identity survives normal OAuth access/refresh token rotation.
    // Without one, hash the credential material rather than sharing a cache
    // across an unidentifiable replacement credential.
    let identity = if !api {
        extract_xai_user_id_from_auth_config(transport.key.decrypted_auth_config.as_deref())
            .or_else(|| transport.key.decrypted_auth_config.clone())
            .unwrap_or_else(|| transport.key.decrypted_api_key.clone())
    } else {
        transport.key.decrypted_api_key.clone()
    };
    let mut digest = Sha256::new();
    for part in [
        "aether:xai-model-catalog:v1",
        &transport.provider.id,
        &transport.key.id,
        &url,
        if api { "api" } else { "oauth" },
        &identity,
    ] {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    // Header overrides can select a different upstream tenant behind a relay.
    // Include them in the digest without storing any header values in the cache.
    digest.update(serde_json::to_vec(&transport.endpoint.header_rules).unwrap_or_default());
    Ok(format!("{:x}", digest.finalize()))
}

/// Whitelist model-directory fields: upstream reflections, credentials, and
/// arbitrary response payloads must never become persistent/admin model cards.
fn parse_catalog(body: &Value) -> Result<Vec<Value>, String> {
    if serde_json::to_vec(body).map_or(true, |b| b.len() > MAX_CATALOG_BYTES) {
        return Err("xAI models response exceeds size limit".to_string());
    }
    if body.get("error").is_some_and(|v| !v.is_null())
        || body.get("has_more").and_then(Value::as_bool) == Some(true)
    {
        return Err("xAI models response is invalid or incomplete".to_string());
    }
    let items = body
        .as_array()
        .or_else(|| body.get("data").and_then(Value::as_array))
        .or_else(|| body.get("models").and_then(Value::as_array))
        .ok_or_else(|| "xAI models response is missing data array".to_string())?;
    if items.is_empty() || items.len() > MAX_MODELS {
        return Err("xAI models response is empty or exceeds model limit".to_string());
    }
    let mut models = BTreeMap::new();
    for item in items {
        let id = text(item, 256)
            .or_else(|| {
                ["id", "model", "name"]
                    .iter()
                    .find_map(|key| text(&item[*key], 256))
            })
            .ok_or_else(|| "xAI models response contains an invalid model ID".to_string())?;
        if item.get("disabled").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let backend = text(&item["api_backend"], 64);
        let Some(format) = api_format(id, backend) else {
            continue;
        };
        let mut card = json!({
            "id": id, "object": "model", "owned_by": "xai", "api_formats": [format],
            "display_name": text(&item["display_name"], 512)
                .or_else(|| text(&item["name"], 512)).unwrap_or(id),
        });
        if let Some(backend) = backend {
            card["api_backend"] = json!(backend);
        }
        let context = positive_number(&item["context_window"])
            .or_else(|| positive_number(&item["context_length"]));
        let mut windows = item
            .get("context_windows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(32)
            .filter_map(positive_number)
            .collect::<BTreeSet<_>>();
        if let Some(context) = context {
            card["context_window"] = json!(context);
            windows.insert(context);
        }
        if let Some(maximum) = windows.last() {
            card["max_context_window"] = json!(maximum);
            card["context_windows"] = json!(windows);
        }
        if let Some(output) = [
            "max_output_tokens",
            "output_token_limit",
            "max_completion_tokens",
        ]
        .iter()
        .find_map(|key| positive_number(&item[*key]))
        {
            card["max_output_tokens"] = json!(output);
        }
        let efforts = item
            .get("reasoning_efforts")
            .or_else(|| item.get("supported_reasoning_efforts"))
            .or_else(|| item.get("supported_reasoning_levels"))
            .and_then(Value::as_array);
        let mut levels = BTreeMap::new();
        let mut default_effort = text(&item["default_reasoning_effort"], 32)
            .or_else(|| text(&item["default_reasoning_level"], 32));
        for effort in efforts.into_iter().flatten().take(32) {
            let Some(value) = text(effort, 32)
                .or_else(|| text(&effort["value"], 32))
                .or_else(|| text(&effort["effort"], 32))
            else {
                continue;
            };
            if !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                continue;
            }
            if effort["default"].as_bool() == Some(true) {
                default_effort = Some(value);
            }
            let description = text(&effort["description"], 512)
                .or_else(|| text(&effort["name"], 512))
                .unwrap_or(value);
            levels.insert((effort_rank(value), value), description);
        }
        if !levels.is_empty() {
            card["reasoning_efforts"] = json!(levels
                .iter()
                .map(|((_, v), d)| { json!({"value": v, "description": d}) })
                .collect::<Vec<_>>());
            card["supported_reasoning_levels"] = json!(levels
                .iter()
                .map(|((_, v), d)| { json!({"effort": v, "description": d}) })
                .collect::<Vec<_>>());
            card["supports_reasoning"] = json!(true);
            if let Some(default) = default_effort.filter(|v| levels.keys().any(|(_, e)| e == v)) {
                card["default_reasoning_level"] = json!(default);
            }
        }
        models.entry(id.to_string()).or_insert(card);
    }
    if models.is_empty() {
        return Err("xAI models response contains no supported models".to_string());
    }
    let cards = models.into_values().collect::<Vec<_>>();
    if serde_json::to_vec(&cards).map_or(true, |b| b.len() > MAX_CATALOG_BYTES) {
        return Err("xAI normalized model directory exceeds size limit".to_string());
    }
    Ok(cards)
}

fn api_format(id: &str, backend: Option<&str>) -> Option<&'static str> {
    match backend {
        Some("responses") => Some("openai:responses"),
        Some("chat" | "chat_completions") => Some("openai:chat"),
        Some("image" | "images" | "image_generation") => Some("openai:image"),
        Some("video" | "videos" | "video_generation") => Some("openai:video"),
        Some(_) => None,
        None if id.starts_with("grok-imagine-video") => Some("openai:video"),
        None if id.starts_with("grok-imagine-image") => Some("openai:image"),
        None => Some("openai:responses"),
    }
}

fn text(value: &Value, limit: usize) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= limit && !s.chars().any(char::is_control))
}

fn positive_number(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .filter(|n| *n > 0 && *n <= i32::MAX as u64)
}

fn effort_rank(effort: &str) -> usize {
    ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
        .iter()
        .position(|e| *e == effort)
        .unwrap_or(usize::MAX)
}

fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
#[path = "xai_tests.rs"]
mod tests;
