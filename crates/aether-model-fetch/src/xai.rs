use std::collections::{BTreeMap, BTreeSet};

use aether_data_contracts::repository::provider_catalog::StoredProviderCatalogKey;
use aether_provider_transport::xai::{
    extract_xai_user_id_from_auth_config, insert_cli_identity_headers,
    should_attach_cli_identity_headers,
};
use aether_provider_transport::GatewayProviderTransportSnapshot;
use serde_json::{json, Value};

use crate::logic::{normalize_cached_model, ModelsFetchPage};

const XAI_MEDIA_MODEL_CARDS: &[(&str, &str, &str)] = &[
    ("grok-imagine-image", "Grok Imagine Image", "openai:image"),
    (
        "grok-imagine-image-quality",
        "Grok Imagine Image Quality",
        "openai:image",
    ),
    ("grok-imagine-video", "Grok Imagine Video", "openai:video"),
    (
        "grok-imagine-video-1.5",
        "Grok Imagine Video 1.5",
        "openai:video",
    ),
];

/// Return the xAI media models that are supported by the key's configured API
/// formats but may be omitted from the account `/v1/models` directory.
pub fn media_models_for_key(key: &StoredProviderCatalogKey) -> Vec<Value> {
    let formats = crate::logic::json_string_list(key.api_formats.as_ref())
        .into_iter()
        .map(|format| aether_ai_formats::normalize_api_format_alias(&format))
        .collect::<std::collections::BTreeSet<_>>();
    XAI_MEDIA_MODEL_CARDS
        .iter()
        .filter(|(_, _, api_format)| formats.is_empty() || formats.contains(*api_format))
        .map(|(id, display_name, api_format)| {
            let mut model = json!({
                "id": id,
                "display_name": display_name,
                "owned_by": "xai",
                "api_formats": [api_format],
            });
            if *api_format == "openai:image" {
                model["supports_image_generation"] = json!(true);
            }
            model
        })
        .collect()
}

pub fn media_model_ids_for_key(key: &StoredProviderCatalogKey) -> Vec<String> {
    media_models_for_key(key)
        .into_iter()
        .filter_map(|model| {
            model
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect()
}

pub(crate) fn models_fetch_headers(
    transport: &GatewayProviderTransportSnapshot,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([("accept".to_string(), "application/json".to_string())]);
    if should_attach_cli_identity_headers(transport, "openai:responses") {
        insert_cli_identity_headers(&mut headers);
        headers.insert("x-grok-client-mode".to_string(), "interactive".to_string());
        let auth_config = transport.key.decrypted_auth_config.as_deref();
        if let Some(user_id) = extract_xai_user_id_from_auth_config(auth_config) {
            headers.insert("x-userid".to_string(), user_id);
        }
        if let Some(email) = auth_config
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .and_then(|config| {
                config
                    .get("email")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .map(|email| email.trim().to_string())
            .filter(|email| !email.is_empty())
        {
            headers.insert("x-email".to_string(), email);
        }
    }
    headers
}

pub(crate) fn parse_models_response(
    endpoint_api_format: &str,
    body: &Value,
) -> Result<ModelsFetchPage, String> {
    let items = body
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| body.get("models").and_then(Value::as_array))
        .or_else(|| body.as_array())
        .ok_or_else(|| "xAI models response is missing data array".to_string())?;
    let mut seen = BTreeSet::new();
    let mut fetched_model_ids = Vec::new();
    let mut cached_models = Vec::new();
    for item in items {
        let Some(model_id) = model_id(item) else {
            continue;
        };
        if !seen.insert(model_id.to_string()) {
            continue;
        }
        // The account directory includes media models even when fetched through
        // a Responses endpoint. Keep their protocol separate in the admin cache.
        let api_format = if model_id.starts_with("grok-imagine-image") {
            "openai:image"
        } else if model_id.starts_with("grok-imagine-video") {
            "openai:video"
        } else if aether_ai_formats::normalize_api_format_alias(endpoint_api_format)
            == "openai:chat"
        {
            "openai:chat"
        } else {
            "openai:responses"
        };
        let mut model = normalize_cached_model(item, model_id, api_format);
        let object = model
            .as_object_mut()
            .expect("normalized model is an object");
        object.entry("owned_by").or_insert_with(|| json!("xai"));
        if !object.contains_key("display_name") {
            if let Some(name) = item.get("name").and_then(Value::as_str) {
                object.insert("display_name".to_string(), json!(name));
            }
        }
        fetched_model_ids.push(model_id.to_string());
        cached_models.push(model);
    }
    // Treat an empty or unrecognized directory as a failed sync so the worker
    // retains the previous whitelist instead of replacing it with an empty one.
    if cached_models.is_empty() {
        return Err("xAI models response contains no models".to_string());
    }
    Ok(ModelsFetchPage {
        fetched_model_ids,
        cached_models,
        has_more: false,
        next_after_id: None,
    })
}

fn model_id(item: &Value) -> Option<&str> {
    fn normalize(value: &Value) -> Option<&str> {
        value
            .as_str()
            .map(str::trim)
            .map(|id| id.strip_prefix("models/").unwrap_or(id))
            .filter(|id| !id.is_empty())
    }
    if item.is_string() {
        return normalize(item);
    }
    // Grok's `id`/`name` may be presentation fields. Prefer the protocol model
    // identity, including the CLI catalog's camelCase and _meta variants.
    ["model", "modelId", "model_id", "id", "slug"]
        .into_iter()
        .filter_map(|field| item.get(field))
        .chain(
            ["model", "modelId", "model_id", "id", "slug", "name"]
                .into_iter()
                .filter_map(|field| item.get("_meta").and_then(|meta| meta.get(field))),
        )
        .chain(item.get("name"))
        .find_map(normalize)
}

#[cfg(test)]
mod tests {
    use super::{media_models_for_key, parse_models_response};
    use aether_data_contracts::repository::provider_catalog::StoredProviderCatalogKey;
    use serde_json::json;

    fn key_with_formats(formats: &[&str]) -> StoredProviderCatalogKey {
        let mut key = StoredProviderCatalogKey::new(
            "key-1".to_string(),
            "provider-1".to_string(),
            "key".to_string(),
            "oauth".to_string(),
            None,
            true,
        )
        .expect("key should build");
        key.api_formats = Some(json!(formats));
        key
    }

    #[test]
    fn xai_media_fallback_follows_key_api_formats() {
        let models = media_models_for_key(&key_with_formats(&["openai:responses", "openai:image"]));
        let ids = models
            .iter()
            .filter_map(|model| model.get("id").and_then(|value| value.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(ids, ["grok-imagine-image", "grok-imagine-image-quality"]);
    }

    #[test]
    fn xai_catalog_prefers_protocol_ids_and_keeps_display_names() {
        let parsed = parse_models_response(
            "openai:responses",
            &json!({"models": [
                {"id": "display-id", "model": "grok-4.7", "name": "Grok 4.7"},
                {"modelId": "grok-build-new"},
                {"model_id": "grok-composer-new"},
                {"name": "Display Name", "_meta": {"model": "grok-meta"}},
                {"id": "grok-safe", "_meta": "not-an-object"},
                {"slug": "grok-slug"},
                {"name": "models/grok-name"},
                "models/grok-string",
                {"model": "grok-4.7"},
                {}, {"id": "models/"}
            ]}),
        )
        .expect("CLI model directory should parse");
        assert_eq!(
            parsed.fetched_model_ids,
            vec![
                "grok-4.7",
                "grok-build-new",
                "grok-composer-new",
                "grok-meta",
                "grok-safe",
                "grok-slug",
                "grok-name",
                "grok-string",
            ]
        );
        assert_eq!(parsed.cached_models[0]["display_name"], "Grok 4.7");
        assert_eq!(parsed.cached_models[0]["id"], "grok-4.7");
    }

    #[test]
    fn xai_catalog_preserves_media_formats_in_all_supported_envelopes() {
        let models = json!([
            {"id": "grok-4.7"},
            {"id": "grok-imagine-image-2.0"},
            {"id": "grok-imagine-video-1.5"}
        ]);
        for body in [json!({"data": models}), json!({"models": models}), models] {
            let parsed = parse_models_response("openai:responses:compact", &body).unwrap();
            assert_eq!(
                parsed.cached_models[0]["api_formats"],
                json!(["openai:responses"])
            );
            assert_eq!(
                parsed.cached_models[1]["api_formats"],
                json!(["openai:image"])
            );
            assert_eq!(
                parsed.cached_models[2]["api_formats"],
                json!(["openai:video"])
            );
        }
    }

    #[test]
    fn xai_catalog_rejects_empty_or_invalid_responses() {
        for body in [
            json!({}),
            json!({"data": []}),
            json!({"models": [{"name": " "}]}),
            json!([]),
        ] {
            assert!(parse_models_response("openai:responses", &body).is_err());
        }
    }
}
