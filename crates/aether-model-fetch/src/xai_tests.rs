use super::*;
use std::sync::Mutex;

use aether_contracts::{ExecutionPlan, ExecutionResult, ProxySnapshot, ResponseBody};
use aether_provider_transport::snapshot::{
    GatewayProviderTransportEndpoint, GatewayProviderTransportKey, GatewayProviderTransportProvider,
};
use aether_provider_transport::LocalResolvedOAuthRequestAuth;
use async_trait::async_trait;

struct Runtime {
    plans: Mutex<Vec<ExecutionPlan>>,
    response: Mutex<(u16, Value)>,
}

impl Runtime {
    fn new(status: u16, body: Value) -> Self {
        Self {
            plans: Mutex::new(Vec::new()),
            response: Mutex::new((status, body)),
        }
    }
}

#[async_trait]
impl ModelFetchTransportRuntime for Runtime {
    async fn resolve_local_oauth_request_auth(
        &self,
        transport: &GatewayProviderTransportSnapshot,
    ) -> Result<Option<LocalResolvedOAuthRequestAuth>, String> {
        Ok(
            (transport.key.auth_type == "oauth").then(|| LocalResolvedOAuthRequestAuth::Header {
                name: "authorization".to_string(),
                value: "Bearer refreshed-token".to_string(),
            }),
        )
    }

    async fn resolve_model_fetch_proxy(
        &self,
        _: &GatewayProviderTransportSnapshot,
    ) -> Option<ProxySnapshot> {
        Some(ProxySnapshot {
            url: Some("http://proxy.example:8080".into()),
            ..Default::default()
        })
    }

    async fn execute_model_fetch_execution_plan(
        &self,
        plan: &ExecutionPlan,
    ) -> Result<ExecutionResult, String> {
        self.plans.lock().unwrap().push(plan.clone());
        let (status, body) = self.response.lock().unwrap().clone();
        Ok(ExecutionResult {
            request_id: plan.request_id.clone(),
            candidate_id: None,
            status_code: status,
            headers: BTreeMap::new(),
            response_observation: None,
            body: Some(ResponseBody {
                json_body: Some(body),
                body_bytes_b64: None,
            }),
            telemetry: None,
            error: None,
        })
    }
}

fn transport(auth_type: &str) -> GatewayProviderTransportSnapshot {
    GatewayProviderTransportSnapshot {
        provider: GatewayProviderTransportProvider {
            id: "p-xai".into(),
            name: "xAI".into(),
            provider_type: "xai".into(),
            website: None,
            is_active: true,
            keep_priority_on_conversion: false,
            enable_format_conversion: false,
            concurrent_limit: None,
            max_retries: None,
            proxy: None,
            request_timeout_secs: Some(10.0),
            stream_first_byte_timeout_secs: None,
            config: None,
        },
        endpoint: GatewayProviderTransportEndpoint {
            id: "e-xai".into(),
            provider_id: "p-xai".into(),
            api_format: "openai:responses".into(),
            api_family: None,
            endpoint_kind: None,
            is_active: true,
            base_url: "https://api.x.ai/v1".into(),
            header_rules: None,
            body_rules: None,
            max_retries: None,
            custom_path: None,
            config: None,
            format_acceptance_config: None,
            proxy: None,
        },
        key: GatewayProviderTransportKey {
            id: "k-xai".into(),
            provider_id: "p-xai".into(),
            name: "account".into(),
            auth_type: auth_type.into(),
            is_active: true,
            api_formats: Some(vec!["openai:responses".into()]),
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
            decrypted_api_key: "secret-one".into(),
            decrypted_auth_config: (auth_type == "oauth")
                .then(|| r#"{"sub":"account-one","refresh_token":"refresh-one"}"#.into()),
        },
    }
}

#[test]
fn xai_parses_windows_efforts_and_only_directory_fields() {
    let cards = parse_catalog(&json!({"data": [{
        "id": "grok-4.7", "name": "Grok 4.7", "api_backend": "responses",
        "context_window": "256000", "context_windows": [500000, 256000, 0, -1, "500000"],
        "max_output_tokens": 32000,
        "reasoning_efforts": [
            {"value":"xhigh", "name":"Extra high"},
            {"value":"high", "default":true},
            {"value":"low", "description":"Low effort"},
            {"value":"bad\nvalue"}
        ],
        "access_token":"must-not-persist", "headers":{"authorization":"must-not-persist"}
    }]}))
    .unwrap();
    let card = &cards[0];
    assert_eq!(card["context_window"], 256000);
    assert_eq!(card["context_windows"], json!([256000, 500000]));
    assert_eq!(card["max_context_window"], 500000);
    assert_eq!(
        card["supported_reasoning_levels"][0],
        json!({"effort":"low","description":"Low effort"})
    );
    assert_eq!(card["supported_reasoning_levels"][2]["effort"], "xhigh");
    assert_eq!(card["default_reasoning_level"], "high");
    assert_eq!(parse_catalog(&json!(cards)).unwrap(), cards);
    assert!(!serde_json::to_string(card)
        .unwrap()
        .contains("must-not-persist"));
}

#[test]
fn xai_accepts_list_shapes_and_maps_media_without_inventing_window_aliases() {
    for body in [
        json!({"models":[{"model":"grok-4.7"}]}),
        json!(["grok-4.7", "grok-4.7"]),
        json!({"data":[{"id":"grok-4.7"}]}),
    ] {
        let cards = parse_catalog(&body).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["id"], "grok-4.7");
    }
    let cards = parse_catalog(&json!({"data":[
        {"id":"grok-imagine-image-2.0"}, {"id":"grok-imagine-video-1.5-lite"},
        {"id":"grok-chat", "api_backend":"chat"},
        {"id":"disabled", "disabled":true}, {"id":"unsupported", "api_backend":"unknown"}
    ]}))
    .unwrap();
    assert_eq!(cards.len(), 3);
    assert_eq!(cards[0]["api_formats"], json!(["openai:chat"]));
    assert_eq!(cards[1]["api_formats"], json!(["openai:image"]));
    assert_eq!(cards[2]["api_formats"], json!(["openai:video"]));
}

#[test]
fn xai_rejects_empty_partial_and_error_payloads_before_replacing_a_cache() {
    for body in [
        json!({}),
        json!({"data":[]}),
        json!({"data":[{"id":"valid"},{}]}),
        json!({"data":[{"id":"valid"}],"error":{"message":"secret"}}),
        json!({"data":[{"id":"valid"}],"has_more":true}),
        json!({"data":[null]}),
        json!({"data": vec!["grok"; MAX_MODELS + 1]}),
    ] {
        assert!(parse_catalog(&body).is_err(), "{body}");
    }
}

#[tokio::test]
async fn xai_oauth_uses_account_models_and_cli_headers_with_the_execution_proxy() {
    let runtime = Runtime::new(200, json!({"data":[{"id":"grok-account-only"}]}));
    let account = transport("oauth");
    let outcome = crate::fetch_models_from_transports(&runtime, &[account])
        .await
        .unwrap();
    assert!(outcome.has_success);
    assert_eq!(outcome.fetched_model_ids, ["grok-account-only"]);
    assert_eq!(outcome.xai_source, Some(XaiModelCatalogSource::Upstream));
    let plans = runtime.plans.lock().unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].url, "https://cli-chat-proxy.grok.com/v1/models");
    assert_eq!(plans[0].headers["authorization"], "Bearer refreshed-token");
    assert_eq!(plans[0].headers["x-xai-token-auth"], "xai-grok-cli");
    assert_eq!(plans[0].headers["x-userid"], "account-one");
    assert_eq!(
        plans[0].headers["x-grok-client-version"],
        aether_provider_transport::xai::xai_client_version()
    );
    assert_eq!(
        plans[0].proxy.as_ref().unwrap().url.as_deref(),
        Some("http://proxy.example:8080")
    );
    assert_eq!(plans[0].timeouts.as_ref().unwrap().total_ms, Some(10000));
}

#[tokio::test]
async fn xai_api_using_api_and_custom_gateways_choose_the_right_models_endpoint() {
    for (auth, config, base, expected) in [
        (
            "api_key",
            None,
            "https://cli-chat-proxy.grok.com/v1",
            "https://api.x.ai/v1/models",
        ),
        (
            "oauth",
            Some(r#"{"using_api":true,"sub":"account-one"}"#),
            "",
            "https://api.x.ai/v1/models",
        ),
        (
            "api_key",
            None,
            "https://relay.example/custom/v1",
            "https://relay.example/custom/v1/models",
        ),
    ] {
        let runtime = Runtime::new(200, json!({"data":[{"id":"grok-new"}]}));
        let mut account = transport(auth);
        account.endpoint.base_url = base.into();
        account.endpoint.api_format = "openai:responses:compact".into();
        account.key.decrypted_auth_config = config.map(str::to_string);
        fetch_xai_models_from_transports(&runtime, &[account], true)
            .await
            .unwrap();
        let plans = runtime.plans.lock().unwrap();
        assert_eq!(plans[0].url, expected);
        assert!(!plans[0].headers.contains_key("x-xai-token-auth"));
        assert!(!plans[0].headers.contains_key("x-userid"));
        assert!(!plans[0].headers.contains_key("originator"));
    }
}

#[tokio::test]
async fn xai_last_success_survives_failures_without_becoming_a_new_success() {
    let runtime = Runtime::new(
        200,
        json!({"data":[{"id":"grok-private","context_window":128000}]}),
    );
    let mut account = transport("oauth");
    let success = fetch_xai_models_from_transports(&runtime, &[account.clone()], true)
        .await
        .unwrap();
    account.key.upstream_metadata = success.upstream_metadata.clone();
    let saved = account.key.upstream_metadata.clone();
    // A new call sees the metadata as persisted in the account row (including after restart).
    let failing = Runtime::new(503, json!({"error":{"message":"Bearer leaked-secret"}}));
    let outcome = fetch_xai_models_from_transports(&failing, &[account.clone()], true)
        .await
        .unwrap();
    assert_eq!(outcome.fetched_model_ids, ["grok-private"]);
    assert_eq!(
        outcome.xai_source,
        Some(XaiModelCatalogSource::LastSuccessCache)
    );
    assert!(!outcome.has_success);
    assert!(outcome.upstream_metadata.is_none());
    assert!(!outcome.errors.join("").contains("leaked-secret"));
    assert_eq!(account.key.upstream_metadata, saved);
    *failing.response.lock().unwrap() = (200, json!({"data":[]}));
    let empty = fetch_xai_models_from_transports(&failing, &[account], true)
        .await
        .unwrap();
    assert_eq!(empty.fetched_model_ids, ["grok-private"]);
    assert!(!empty.has_success);
}

#[tokio::test]
async fn xai_fresh_cache_avoids_fetch_but_force_and_staleness_refresh() {
    let runtime = Runtime::new(200, json!({"data":[{"id":"grok-cached"}]}));
    let mut account = transport("api_key");
    let success = fetch_xai_models_from_transports(&runtime, &[account.clone()], true)
        .await
        .unwrap();
    account.key.upstream_metadata = success.upstream_metadata;
    let next = Runtime::new(200, json!({"data":[{"id":"grok-live"}]}));
    let cached = fetch_xai_models_from_transports(&next, &[account.clone()], false)
        .await
        .unwrap();
    assert_eq!(
        cached.xai_source,
        Some(XaiModelCatalogSource::LastSuccessCache)
    );
    assert!(next.plans.lock().unwrap().is_empty());
    let scope = catalog_scope(&account).unwrap();
    account.key.upstream_metadata.as_mut().unwrap()[CATALOG_NAMESPACE]["catalogs"][&scope]
        ["fetched_at"] = json!(current_unix_secs() - FRESH_FOR_SECS - 1);
    let live = fetch_xai_models_from_transports(&next, &[account.clone()], false)
        .await
        .unwrap();
    assert_eq!(live.fetched_model_ids, ["grok-live"]);
    assert_eq!(next.plans.lock().unwrap().len(), 1);
    let forced = fetch_xai_models_from_transports(&next, &[account], true)
        .await
        .unwrap();
    assert!(forced.has_success);
    assert_eq!(next.plans.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn xai_caches_do_not_cross_accounts_credentials_modes_or_entry_points() {
    let live = Runtime::new(200, json!({"data":[{"id":"grok-secret-catalog"}]}));
    let mut account = transport("oauth");
    let success = fetch_xai_models_from_transports(&live, &[account.clone()], true)
        .await
        .unwrap();
    account.key.upstream_metadata = success.upstream_metadata;
    let failing = Runtime::new(401, json!({"error":{"message":"bad token"}}));
    let mut changed = Vec::new();
    let mut key = account.clone();
    key.key.id = "different-key".into();
    changed.push(key);
    let mut subject = account.clone();
    subject.key.decrypted_auth_config = Some(r#"{"sub":"someone-else"}"#.into());
    changed.push(subject);
    let mut gateway = account.clone();
    gateway.endpoint.base_url = "https://other.example/v1".into();
    changed.push(gateway);
    let mut mode = account.clone();
    mode.key.decrypted_auth_config = Some(r#"{"using_api":true,"sub":"account-one"}"#.into());
    changed.push(mode);
    for changed in changed {
        let outcome = fetch_xai_models_from_transports(&failing, &[changed], true)
            .await
            .unwrap();
        assert_eq!(
            outcome.xai_source,
            Some(XaiModelCatalogSource::StaticFallback)
        );
        assert!(!outcome
            .fetched_model_ids
            .contains(&"grok-secret-catalog".to_string()));
        assert!(!outcome.has_success);
        assert!(outcome.upstream_metadata.is_none());
    }
    let scope = catalog_scope(&account).unwrap();
    account.key.decrypted_auth_config =
        Some(r#"{"sub":"account-one","refresh_token":"rotated"}"#.into());
    account.key.decrypted_api_key = "rotated-access".into();
    assert_eq!(catalog_scope(&account).unwrap(), scope);
    let mut api = transport("api_key");
    let scope = catalog_scope(&api).unwrap();
    api.key.decrypted_api_key = "replaced-api-key".into();
    assert_ne!(catalog_scope(&api).unwrap(), scope);
}

#[tokio::test]
async fn xai_rejects_cross_key_merges_and_deduplicates_a_shared_directory() {
    let runtime = Runtime::new(200, json!(["grok-only"]));
    let account = transport("api_key");
    let mut other = account.clone();
    other.key.id = "other".into();
    assert!(
        fetch_xai_models_from_transports(&runtime, &[account.clone(), other], true)
            .await
            .is_err()
    );
    assert!(runtime.plans.lock().unwrap().is_empty());
    let mut same = account.clone();
    same.endpoint.id = "other-format".into();
    same.endpoint.api_format = "claude:messages".into();
    let outcome = fetch_xai_models_from_transports(&runtime, &[account, same], true)
        .await
        .unwrap();
    assert_eq!(outcome.fetched_model_ids, ["grok-only"]);
    assert_eq!(runtime.plans.lock().unwrap().len(), 1);
}
