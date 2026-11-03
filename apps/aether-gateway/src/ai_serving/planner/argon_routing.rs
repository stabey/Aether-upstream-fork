//! Request-scoped Argon variant selection. Never rewrite the client's model or
//! the shared catalog: Vertex and Antigravity can serve the same global model.
use aether_data_contracts::repository::candidate_selection::StoredMinimalCandidateSelectionRow;
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

fn gemini_level(body: &Value) -> Option<ArgonLevel> {
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
        .and_then(ArgonLevel::parse)
}

/// Only explicit three-level inputs participate. Do not approximate a token
/// budget, invent a default, or silently map xhigh/max/none to a different level.
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
    let value = match aether_ai_formats::normalize_api_format_alias(client_format).as_str() {
        "gemini:generate_content" => return gemini_level(body),
        "openai:chat" => body.get("reasoning_effort"),
        "openai:responses" => body.get("reasoning").and_then(|v| v.get("effort")),
        "claude:messages" => {
            if body.pointer("/thinking/type").and_then(Value::as_str) == Some("disabled") {
                return None;
            }
            body.pointer("/output_config/effort")
        }
        _ => None,
    };
    value.and_then(Value::as_str).and_then(ArgonLevel::parse)
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

pub(crate) fn has_variant(allowed: Option<&[String]>, model: &str) -> bool {
    allowed.is_some_and(|models| models.iter().any(|name| name.trim() == model))
}

pub(crate) fn body_matches_variant(model: &str, body: &Value) -> bool {
    gemini_level(body).is_some_and(|level| level.model() == model)
}

pub(crate) fn envelope_matches_level(level: ArgonLevel, envelope: &Value) -> bool {
    let model = level.model();
    envelope.get("model").and_then(Value::as_str) == Some(model.as_str())
        && envelope
            .get("request")
            .is_some_and(|body| body_matches_variant(&model, body))
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
        && is_argon(model)
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
            ("openai:chat", json!({})),
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
}
