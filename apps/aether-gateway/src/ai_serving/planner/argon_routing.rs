//! Request-scoped Argon variant selection. Never rewrite the client's model or
//! the shared catalog: Vertex and Antigravity can serve the same global model.
use aether_data_contracts::repository::candidate_selection::{
    provider_model_mapping_api_format_covers, StoredMinimalCandidateSelectionRow,
    StoredProviderModelMapping,
};
use serde_json::Value;

use crate::system_features::ModelDirectivePolicySnapshot;

pub(crate) fn enabled() -> bool {
    std::env::var("AETHER_ANTIGRAVITY_ARGON_EFFORT_ROUTING").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        )
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArgonLevel {
    Low,
    Medium,
    High,
}

impl ArgonLevel {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub(crate) fn model(self) -> String {
        format!("gemini-4-argon-{}", self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RequestThinkingEffort {
    Unspecified,
    Explicit(String),
}

impl RequestThinkingEffort {
    pub(crate) fn parse_explicit(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized.is_empty() || matches!(normalized.as_str(), "none" | "disabled" | "off") {
            return None;
        }
        if matches!(normalized.as_str(), "unspecified" | "default" | "unset") {
            return Some(Self::Unspecified);
        }
        Some(Self::Explicit(normalized))
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Explicit(value) => value.as_str(),
        }
    }

    pub(crate) fn is_unspecified(&self) -> bool {
        matches!(self, Self::Unspecified)
    }

    pub(crate) fn argon_level(&self, allow_unspecified_default_low: bool) -> Option<ArgonLevel> {
        match self {
            Self::Unspecified => allow_unspecified_default_low.then_some(ArgonLevel::Low),
            Self::Explicit(value) => ArgonLevel::parse(value),
        }
    }
}

fn gemini_thinking_effort_str(body: &Value) -> Option<String> {
    let generation = body
        .get("generationConfig")
        .or_else(|| body.get("generation_config"))?;
    let thinking = generation
        .get("thinkingConfig")
        .or_else(|| generation.get("thinking_config"))?;
    // Budget/level conflicts and budget-only requests keep their existing path.
    if thinking
        .get("thinkingBudget")
        .or_else(|| thinking.get("thinking_budget"))
        .is_some()
    {
        return None;
    }
    thinking
        .get("thinkingLevel")
        .or_else(|| thinking.get("thinking_level"))?
        .as_str()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

fn gemini_level(body: &Value) -> Option<ArgonLevel> {
    gemini_thinking_effort_str(body)
        .as_deref()
        .and_then(ArgonLevel::parse)
}

fn extract_body_thinking_effort(
    normalized_format: &str,
    body: &Value,
) -> Option<RequestThinkingEffort> {
    let value = match normalized_format {
        "gemini:generate_content" => {
            return if gemini_effort_unspecified(body) {
                Some(RequestThinkingEffort::Unspecified)
            } else {
                gemini_thinking_effort_str(body)
                    .as_deref()
                    .and_then(RequestThinkingEffort::parse_explicit)
            };
        }
        "openai:chat" => body.get("reasoning_effort"),
        "openai:responses" => body.get("reasoning").and_then(|v| v.get("effort")),
        "claude:messages" => {
            if body.pointer("/thinking/type").and_then(Value::as_str) == Some("disabled") {
                return None;
            }
            if body.get("thinking").is_some() && body.pointer("/output_config/effort").is_none() {
                return None;
            }
            body.pointer("/output_config/effort")
        }
        _ => return None,
    };
    match value {
        None => Some(RequestThinkingEffort::Unspecified),
        Some(value) => value.as_str().and_then(RequestThinkingEffort::parse_explicit),
    }
}

pub(crate) fn request_thinking_effort(
    policy: &ModelDirectivePolicySnapshot,
    client_format: &str,
    requested_model: &str,
    body: &Value,
) -> Option<RequestThinkingEffort> {
    let normalized_format = aether_ai_formats::normalize_api_format_alias(client_format);
    // Preserve format-native directive effort (including xhigh/max/minimal) before
    // falling back to Gemini/Argon directive resolution.
    let client_directive = policy.resolve_reasoning(&normalized_format, Some(requested_model));
    if client_directive.enabled() {
        let probe_model = match normalized_format.as_str() {
            "gemini:generate_content" => "gemini-4-argon",
            "claude:messages" => "claude-opus-4-6",
            _ => client_directive.base_model().unwrap_or(requested_model),
        };
        if let Ok(Some(patch)) = client_directive.mapping_patch_for_mapped_model(probe_model) {
            let effort = extract_body_thinking_effort(&normalized_format, &patch);
            if effort.as_ref().is_some_and(|e| !e.is_unspecified()) {
                return effort;
            }
        }
    }
    let gemini_directive =
        policy.resolve_reasoning("gemini:generate_content", Some(requested_model));
    match gemini_directive.mapping_patch_for_mapped_model("gemini-4-argon") {
        Ok(Some(patch)) => {
            return gemini_thinking_effort_str(&patch)
                .as_deref()
                .and_then(RequestThinkingEffort::parse_explicit);
        }
        Err(_) => return None,
        Ok(None) => {}
    }
    extract_body_thinking_effort(&normalized_format, body)
}

/// Missing effort defaults to low for eligible Antigravity Argon mappings.
/// Explicit budgets and unsupported levels retain their existing semantics.
pub(crate) fn request_level(
    enabled: bool,
    policy: &ModelDirectivePolicySnapshot,
    client_format: &str,
    requested_model: &str,
    body: &Value,
) -> Option<ArgonLevel> {
    if !enabled {
        return None;
    }
    // Preserve existing suffix/custom-directive precedence over request fields.
    let directive = policy.resolve_reasoning("gemini:generate_content", Some(requested_model));
    match directive.mapping_patch_for_mapped_model("gemini-4-argon") {
        Ok(Some(patch)) => return gemini_level(&patch),
        Err(_) => return None,
        Ok(None) => {}
    }
    let normalized_format = aether_ai_formats::normalize_api_format_alias(client_format);
    extract_body_thinking_effort(&normalized_format, body)?.argon_level(true)
}

fn gemini_effort_unspecified(body: &Value) -> bool {
    let thinking = body
        .get("generationConfig")
        .or_else(|| body.get("generation_config"))
        .and_then(|g| g.get("thinkingConfig").or_else(|| g.get("thinking_config")));
    thinking.is_none_or(|t| {
        t.is_object()
            && [
                "thinkingLevel",
                "thinking_level",
                "thinkingBudget",
                "thinking_budget",
            ]
            .iter()
            .all(|key| t.get(*key).is_none())
    })
}

pub(crate) fn variant_level(model: &str) -> Option<ArgonLevel> {
    if !aether_ai_formats::gemini_model_uses_thinking_level(model) {
        return None;
    }
    let normalized = model.trim().to_ascii_lowercase();
    if normalized.ends_with("-low") {
        Some(ArgonLevel::Low)
    } else if normalized.ends_with("-medium") {
        Some(ArgonLevel::Medium)
    } else if normalized.ends_with("-high") {
        Some(ArgonLevel::High)
    } else {
        None
    }
}

/// Modify only an owned Antigravity body after exact-account routing selected a variant.
pub(crate) fn with_default_variant_level(model: &str, body: &Value) -> Option<Value> {
    let level = variant_level(model)?;
    if !gemini_effort_unspecified(body) {
        return None;
    }
    let mut result = body.clone();
    let root = result.as_object_mut()?;
    let generation_key = if root.contains_key("generationConfig") {
        "generationConfig"
    } else if root.contains_key("generation_config") {
        "generation_config"
    } else {
        "generationConfig"
    };
    let generation = root
        .entry(generation_key)
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()?;
    let thinking_key = if generation.contains_key("thinkingConfig") {
        "thinkingConfig"
    } else if generation.contains_key("thinking_config") {
        "thinking_config"
    } else {
        "thinkingConfig"
    };
    let thinking = generation
        .entry(thinking_key)
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()?;
    thinking.insert(
        "thinkingLevel".into(),
        serde_json::json!(level.as_str().to_ascii_uppercase()),
    );
    Some(result)
}

/// Modify only an owned Antigravity body after exact-account routing selected low.
pub(crate) fn with_default_low(model: &str, body: &Value) -> Option<Value> {
    if variant_level(model) != Some(ArgonLevel::Low) {
        return None;
    }
    with_default_variant_level(model, body)
}

pub(crate) fn is_argon(model: &str) -> bool {
    matches!(
        model,
        "gemini-4-argon" | "gemini-4-argon-low" | "gemini-4-argon-medium" | "gemini-4-argon-high"
    )
}

/// Resolve the configured mapping and its endpoint/operation scope first, but
/// check credentials against the *new* variant, not the configured default.
pub(crate) fn configured_argon(
    row: &StoredMinimalCandidateSelectionRow,
    requested_model: &str,
    format: &str,
    operation: Option<&str>,
) -> bool {
    if !row.provider_type.eq_ignore_ascii_case("antigravity")
        || aether_ai_formats::normalize_api_format_alias(format) != "gemini:generate_content"
    {
        return false;
    }
    let mut unrestricted = row.clone();
    unrestricted.key_allowed_models = None;
    aether_scheduler_core::resolve_provider_model_name_with_model_directives_and_request_operation(
        &unrestricted,
        requested_model,
        format,
        false,
        operation,
    )
    .is_some_and(|(model, _)| is_argon(&model))
}

fn mapping_scope_matches(
    mapping: &StoredProviderModelMapping,
    row: &StoredMinimalCandidateSelectionRow,
    normalized_api_format: &str,
    request_operation: Option<&str>,
) -> bool {
    let api_format_matches = mapping.api_formats.as_ref().is_none_or(|api_formats| {
        api_formats.iter().any(|value| {
            provider_model_mapping_api_format_covers(
                &row.provider_type,
                value,
                normalized_api_format,
            )
        })
    });
    if !api_format_matches {
        return false;
    }
    let endpoint_matches = mapping.endpoint_ids.as_ref().is_none_or(|endpoint_ids| {
        endpoint_ids
            .iter()
            .any(|endpoint_id| endpoint_id == &row.endpoint_id)
    });
    if !endpoint_matches {
        return false;
    }
    mapping.operations.as_ref().is_none_or(|operations| {
        request_operation.is_some_and(|request_operation| {
            operations
                .iter()
                .any(|operation| operation.eq_ignore_ascii_case(request_operation))
        })
    })
}

fn mapping_operation_scope_rank(mapping: &StoredProviderModelMapping) -> u8 {
    u8::from(
        mapping
            .operations
            .as_ref()
            .is_some_and(|operations| !operations.is_empty()),
    )
}

fn mapping_effort_matches(mapping: &StoredProviderModelMapping, target_effort: &str) -> bool {
    mapping
        .thinking_efforts
        .as_ref()
        .is_some_and(|efforts| {
            efforts.iter().any(|item| {
                let normalized = match item.trim().to_ascii_lowercase().as_str() {
                    "default" | "unset" => "unspecified".to_string(),
                    other => other.to_string(),
                };
                normalized == target_effort
            })
        })
}

pub(crate) fn row_has_thinking_effort_mappings(
    row: &StoredMinimalCandidateSelectionRow,
    format: &str,
    operation: Option<&str>,
) -> bool {
    let normalized_format = aether_scheduler_core::normalize_api_format(format);
    row.model_provider_model_mappings
        .as_ref()
        .is_some_and(|mappings| {
            mappings.iter().any(|mapping| {
                mapping_scope_matches(mapping, row, &normalized_format, operation)
                    && mapping
                        .thinking_efforts
                        .as_ref()
                        .is_some_and(|efforts| !efforts.is_empty())
            })
        })
}

pub(crate) fn resolve_row_thinking_effort_model(
    row: &StoredMinimalCandidateSelectionRow,
    requested_model: &str,
    format: &str,
    operation: Option<&str>,
    effort: Option<&RequestThinkingEffort>,
) -> Option<String> {
    let normalized_format = aether_scheduler_core::normalize_api_format(format);
    let mut unrestricted = row.clone();
    unrestricted.key_allowed_models = None;
    if !aether_scheduler_core::row_supports_requested_model_with_model_directives_and_request_operation(
        &unrestricted,
        requested_model,
        &normalized_format,
        false,
        operation,
    ) {
        return None;
    }
    let mappings = row.model_provider_model_mappings.as_ref()?;
    let in_scope = mappings
        .iter()
        .filter(|mapping| mapping_scope_matches(mapping, row, &normalized_format, operation))
        .collect::<Vec<_>>();
    if in_scope.is_empty() {
        return None;
    }
    let pick_best = |candidates: Vec<&StoredProviderModelMapping>| -> Option<String> {
        candidates
            .into_iter()
            .min_by(|left, right| {
                left.priority
                    .cmp(&right.priority)
                    .then_with(|| {
                        mapping_operation_scope_rank(right)
                            .cmp(&mapping_operation_scope_rank(left))
                    })
                    .then(left.name.cmp(&right.name))
            })
            .map(|mapping| mapping.name.clone())
    };
    if let Some(effort) = effort {
        let exact_matches = in_scope
            .iter()
            .copied()
            .filter(|mapping| mapping_effort_matches(mapping, effort.as_str()))
            .collect::<Vec<_>>();
        if let Some(selected) = pick_best(exact_matches) {
            return Some(selected);
        }
        if effort.is_unspecified() {
            let low_matches = in_scope
                .iter()
                .copied()
                .filter(|mapping| mapping_effort_matches(mapping, "low"))
                .collect::<Vec<_>>();
            if let Some(selected) = pick_best(low_matches) {
                return Some(selected);
            }
        }
    }
    let unscoped_matches = in_scope
        .into_iter()
        .filter(|mapping| {
            mapping
                .thinking_efforts
                .as_ref()
                .is_none_or(|efforts| efforts.is_empty())
        })
        .collect::<Vec<_>>();
    pick_best(unscoped_matches)
}

pub(crate) fn has_variant(allowed: Option<&[String]>, model: &str) -> bool {
    allowed.is_some_and(|models| models.iter().any(|name| name.trim() == model))
}

pub(crate) fn body_matches_variant(model: &str, body: &Value) -> bool {
    match variant_level(model) {
        Some(expected) => gemini_level(body) == Some(expected),
        None => true,
    }
}

pub(crate) fn envelope_matches_variant(model: &str, envelope: &Value) -> bool {
    envelope.get("model").and_then(Value::as_str) == Some(model)
        && envelope
            .get("request")
            .is_some_and(|body| body_matches_variant(model, body))
}

pub(crate) fn envelope_matches_level(level: ArgonLevel, envelope: &Value) -> bool {
    envelope_matches_variant(&level.model(), envelope)
}

pub(crate) fn select_variant(row: &mut StoredMinimalCandidateSelectionRow, model: &str) {
    // Scope has already been evaluated for this row. The owned copy is used
    // only for this request, before permission checks, quota checks and ranking.
    row.model_provider_model_name = model.to_string();
    row.model_provider_model_mappings = None;
    row.global_model_mappings = None;
}

/// Recheck the concrete variant against the current account snapshot, including
/// pool/sticky-account expansion. A global-model alias cannot grant a variant.
pub(crate) fn transport_skip_reason(
    transport: &crate::ai_serving::GatewayProviderTransportSnapshot,
    model: &str,
    requires_exact_model_permission: bool,
) -> Option<&'static str> {
    exact_permission_skip_reason(
        &transport.provider.provider_type,
        transport.key.allowed_models.as_deref(),
        model,
        requires_exact_model_permission,
    )
}

fn exact_permission_skip_reason(
    provider_type: &str,
    allowed: Option<&[String]>,
    model: &str,
    requires_exact_model_permission: bool,
) -> Option<&'static str> {
    (requires_exact_model_permission
        && provider_type.eq_ignore_ascii_case("antigravity")
        && !has_variant(allowed, model))
    .then_some("argon_variant_not_allowed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn inputs(level: &str) -> Vec<(&'static str, Value)> {
        vec![
            (
                "openai:chat",
                json!({"model":"gemini-4-argon", "messages":[{"role":"user","content":"hello"}], "reasoning_effort":level}),
            ),
            (
                "openai:responses",
                json!({"model":"gemini-4-argon", "input":"hello", "reasoning":{"effort":level}}),
            ),
            (
                "claude:messages",
                json!({"model":"gemini-4-argon", "max_tokens":4096, "messages":[{"role":"user","content":"hello"}], "thinking":{"type":"adaptive"}, "output_config":{"effort":level}}),
            ),
            (
                "gemini:generate_content",
                json!({"contents":[{"role":"user","parts":[{"text":"hello"}]}], "generationConfig":{"thinkingConfig":{"thinkingLevel":level}}}),
            ),
        ]
    }

    #[test]
    fn argon_explicit_levels_match_converted_bodies_for_both_provider_forms() {
        let policy = ModelDirectivePolicySnapshot::default();
        for level in [ArgonLevel::Low, ArgonLevel::Medium, ArgonLevel::High] {
            for (format, body) in inputs(level.as_str()) {
                assert_eq!(
                    request_level(true, &policy, format, "gemini-4-argon", &body),
                    Some(level)
                );
                // Antigravity chooses a suffixed ID; Vertex keeps its mapped ID.
                for mapped_model in [level.model(), "gemini-4-argon".to_string()] {
                    let output = aether_ai_formats::convert_request(
                        format,
                        "gemini:generate_content",
                        &body,
                        &aether_ai_formats::FormatContext::default()
                            .with_mapped_model(&mapped_model),
                    )
                    .unwrap();
                    assert_eq!(
                        output.pointer("/generationConfig/thinkingConfig/thinkingLevel"),
                        Some(&json!(level.as_str())),
                        "{format}: {output}"
                    );
                }
            }
        }
    }

    #[test]
    fn argon_disabled_unspecified_budget_and_unsupported_efforts_do_not_select_variants() {
        let policy = ModelDirectivePolicySnapshot::default();
        for (format, body) in inputs("medium") {
            assert_eq!(
                request_level(false, &policy, format, "gemini-4-argon", &body),
                None
            );
        }
        for (format, body) in [
            ("openai:chat", json!({"reasoning_effort":"xhigh"})),
            ("openai:responses", json!({"reasoning":{"effort":"none"}})),
            (
                "claude:messages",
                json!({"thinking":{"type":"enabled","budget_tokens":8192}}),
            ),
            (
                "claude:messages",
                json!({"thinking":{"type":"disabled"},"output_config":{"effort":"low"}}),
            ),
            (
                "gemini:generate_content",
                json!({"generationConfig":{"thinkingConfig":{"thinkingBudget":8192}}}),
            ),
            (
                "gemini:generate_content",
                json!({"generationConfig":{"thinkingConfig":{"thinkingBudget":8192,"thinkingLevel":"low"}}}),
            ),
        ] {
            assert_eq!(
                request_level(true, &policy, format, "gemini-4-argon", &body),
                None,
                "{format} {body}"
            );
        }
    }

    #[test]
    fn argon_missing_effort_defaults_low_without_mutating_shared_body() {
        let policy = ModelDirectivePolicySnapshot::default();
        for format in [
            "gemini:generate_content",
            "openai:chat",
            "openai:responses",
            "claude:messages",
        ] {
            assert_eq!(
                request_level(true, &policy, format, "gemini-4-argon", &json!({})),
                Some(ArgonLevel::Low)
            );
        }
        let body =
            json!({"tools":[{"googleSearch":{}}],"generationConfig":{"maxOutputTokens":100}});
        let patched = with_default_low("gemini-4-argon-low", &body).unwrap();
        assert!(body_matches_variant("gemini-4-argon-low", &patched));
        assert_eq!(patched["tools"], body["tools"]);
        assert_eq!(patched["generationConfig"]["maxOutputTokens"], 100);
        assert!(body.pointer("/generationConfig/thinkingConfig").is_none());
        assert!(with_default_low("gemini-4-argon-high", &body).is_none());
        for body in [
            json!({"generationConfig":{"thinkingConfig":{"thinkingBudget":0}}}),
            json!({"generationConfig":{"thinkingConfig":{"thinkingLevel":"HIGH"}}}),
        ] {
            assert!(with_default_low("gemini-4-argon-low", &body).is_none());
        }
    }

    #[test]
    fn argon_suffix_directives_keep_their_existing_precedence() {
        let policy = ModelDirectivePolicySnapshot::from_config_values(Some(&json!(true)), None);
        let body = json!({"reasoning_effort":"low"});
        assert_eq!(
            request_level(true, &policy, "openai:chat", "gemini-4-argon-high", &body),
            Some(ArgonLevel::High)
        );
        let policy = ModelDirectivePolicySnapshot::default();
        assert_eq!(
            request_level(true, &policy, "openai:chat", "gemini-4-argon-high", &body),
            Some(ArgonLevel::Low)
        );
    }

    #[test]
    fn argon_native_snake_case_level_is_recognized() {
        assert_eq!(
            request_level(
                true,
                &ModelDirectivePolicySnapshot::default(),
                "gemini:generate_content",
                "gemini-4-argon",
                &json!({"generation_config":{"thinking_config":{"thinking_level":"MEDIUM"}}})
            ),
            Some(ArgonLevel::Medium)
        );
    }

    #[test]
    fn argon_outbound_contract_rejects_conflicting_provider_rules() {
        let body = json!({"generationConfig":{"thinkingConfig":{"thinkingLevel":"low"}}});
        assert!(body_matches_variant("gemini-4-argon-low", &body));
        assert!(!body_matches_variant("gemini-4-argon-high", &body));
        assert!(!body_matches_variant("gemini-4-argon-low", &json!({})));
        assert!(!has_variant(
            Some(&["gemini-4-argon".to_string()]),
            "gemini-4-argon-low"
        ));
        assert!(!has_variant(None, "gemini-4-argon-low"));
        let envelope = json!({"model":"gemini-4-argon-low", "request":body});
        assert!(envelope_matches_level(ArgonLevel::Low, &envelope));
        assert!(!envelope_matches_level(ArgonLevel::High, &envelope));
        assert!(!envelope_matches_level(
            ArgonLevel::Low,
            &json!({"model":"gemini-4-argon-high", "request":body})
        ));
    }

    #[test]
    fn argon_concrete_account_recheck_does_not_borrow_pool_or_alias_permissions() {
        let high_only = vec![
            "gemini-4-argon-high".to_string(),
            "gemini-4-argon".to_string(),
        ];
        assert_eq!(
            exact_permission_skip_reason(
                "antigravity",
                Some(&high_only),
                "gemini-4-argon-low",
                true
            ),
            Some("argon_variant_not_allowed")
        );
        assert_eq!(
            exact_permission_skip_reason(
                "antigravity",
                Some(&high_only),
                "gemini-4-argon-high",
                true
            ),
            None
        );
        assert_eq!(
            exact_permission_skip_reason("antigravity", None, "gemini-4-argon-low", true),
            Some("argon_variant_not_allowed")
        );
        assert_eq!(
            exact_permission_skip_reason("antigravity", None, "gemini-4-argon-low", false),
            None
        );
        assert_eq!(
            exact_permission_skip_reason("vertex_ai", None, "gemini-4-argon", false),
            None
        );
    }

    #[test]
    fn generic_thinking_effort_mappings_resolve_variants_and_default_unspecified_to_low() {
        let row = StoredMinimalCandidateSelectionRow {
            provider_id: "anti".to_string(),
            provider_name: "anti".to_string(),
            provider_type: "antigravity".to_string(),
            provider_priority: 1,
            provider_is_active: true,
            endpoint_id: "ep-anti".to_string(),
            endpoint_api_format: "gemini:generate_content".to_string(),
            endpoint_api_family: Some("gemini".to_string()),
            endpoint_kind: Some("generate_content".to_string()),
            endpoint_is_active: true,
            key_id: "key-anti".to_string(),
            key_name: "key-anti".to_string(),
            key_auth_type: "api_key".to_string(),
            key_is_active: true,
            key_api_formats: Some(vec!["gemini:generate_content".to_string()]),
            key_allowed_models: Some(vec![
                "gemini-3.8-flash-low".to_string(),
                "gemini-3.8-flash-medium".to_string(),
            ]),
            key_capabilities: None,
            key_internal_priority: 1,
            key_global_priority_by_format: None,
            model_id: "model-anti".to_string(),
            global_model_id: "gm-flash".to_string(),
            global_model_name: "gemini-3.8-flash".to_string(),
            global_model_mappings: None,
            global_model_supports_streaming: Some(true),
            model_provider_model_name: "gemini-3.8-flash".to_string(),
            model_provider_model_mappings: Some(vec![
                StoredProviderModelMapping {
                    name: "gemini-3.8-flash-low".to_string(),
                    priority: 1,
                    api_formats: Some(vec!["gemini:generate_content".to_string()]),
                    endpoint_ids: None,
                    operations: None,
                    thinking_efforts: Some(vec!["low".to_string()]),
                },
                StoredProviderModelMapping {
                    name: "gemini-3.8-flash-medium".to_string(),
                    priority: 2,
                    api_formats: Some(vec!["gemini:generate_content".to_string()]),
                    endpoint_ids: None,
                    operations: None,
                    thinking_efforts: Some(vec!["medium".to_string()]),
                },
                StoredProviderModelMapping {
                    name: "gemini-3.8-flash-high".to_string(),
                    priority: 3,
                    api_formats: Some(vec!["gemini:generate_content".to_string()]),
                    endpoint_ids: None,
                    operations: None,
                    thinking_efforts: Some(vec!["high".to_string()]),
                },
            ]),
            model_supports_streaming: Some(true),
            model_is_active: true,
            model_is_available: true,
        };
        assert!(row_has_thinking_effort_mappings(
            &row,
            "gemini:generate_content",
            None
        ));
        assert_eq!(
            resolve_row_thinking_effort_model(
                &row,
                "gemini-3.8-flash",
                "gemini:generate_content",
                None,
                Some(&RequestThinkingEffort::Explicit("medium".to_string())),
            )
            .as_deref(),
            Some("gemini-3.8-flash-medium")
        );
        assert_eq!(
            resolve_row_thinking_effort_model(
                &row,
                "gemini-3.8-flash",
                "gemini:generate_content",
                None,
                Some(&RequestThinkingEffort::Unspecified),
            )
            .as_deref(),
            Some("gemini-3.8-flash-low")
        );
        assert_eq!(
            resolve_row_thinking_effort_model(
                &row,
                "gemini-3.8-flash",
                "gemini:generate_content",
                None,
                Some(&RequestThinkingEffort::Explicit("xhigh".to_string())),
            ),
            None
        );
        let patched = with_default_variant_level("gemini-3.8-flash-medium", &json!({})).unwrap();
        assert!(body_matches_variant("gemini-3.8-flash-medium", &patched));
        assert!(!body_matches_variant("gemini-3.8-flash-low", &patched));
    }
}
