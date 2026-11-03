use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

/// Request URLs legitimately carry API-key and SSE query parameters. Validate
/// their origin with the base-URL policy without weakening that policy itself.
pub fn is_vertex_request_url(raw: &str) -> bool {
    let Ok(mut url) = url::Url::parse(raw) else {
        return false;
    };
    if url.fragment().is_some() {
        return false;
    }
    url.set_query(None);
    super::looks_like_vertex_ai_host(url.as_str())
}

/// RESOURCE_EXHAUSTED is also used for shared serving capacity. Only explicit
/// quota evidence is a quota failure; the status name alone is not evidence.
pub fn vertex_error_has_quota_evidence(body: Option<&str>) -> bool {
    let body = body.unwrap_or_default();
    let lower = body.to_ascii_lowercase();
    if [
        "quota",
        "rate_limit_exceeded",
        "ratelimitexceeded",
        "provisioned throughput",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return true;
    }
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/details")
                .and_then(Value::as_array)
                .cloned()
        })
        .is_some_and(|details| {
            details.iter().any(|detail| {
                detail
                    .get("@type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind.ends_with("google.rpc.QuotaFailure"))
            })
        })
}

/// Bounded delay for the two pre-response capacity retries. A server hint is
/// honored up to eight seconds; later failures go through normal failover.
pub fn vertex_capacity_retry_delay(
    status: u16,
    headers: &BTreeMap<String, String>,
    body: Option<&str>,
    retry_index: usize,
) -> Option<Duration> {
    if status != 429 || retry_index >= 2 || vertex_error_has_quota_evidence(body) {
        return None;
    }
    let value: Value = serde_json::from_str(body?).ok()?;
    if value.pointer("/error/status").and_then(Value::as_str) != Some("RESOURCE_EXHAUSTED") {
        return None;
    }
    // Structured QuotaFailure and other details must not be mistaken for the
    // documented unstructured shared-capacity error. RetryInfo is safe.
    if value
        .pointer("/error/details")
        .and_then(Value::as_array)
        .is_some_and(|details| {
            details.iter().any(|detail| {
                detail.get("@type").and_then(Value::as_str)
                    != Some("type.googleapis.com/google.rpc.RetryInfo")
            })
        })
    {
        return None;
    }
    let message = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ![
        "resource exhausted",
        "resource has been exhausted",
        "try again later",
    ]
    .iter()
    .any(|marker| message.contains(marker))
    {
        return None;
    }
    let retry_after = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, value)| {
            value.trim().parse::<f64>().ok().or_else(|| {
                chrono::DateTime::parse_from_rfc2822(value.trim())
                    .ok()
                    .map(|date| (date.timestamp() - chrono::Utc::now().timestamp()).max(0) as f64)
            })
        });
    let retry_info = value
        .pointer("/error/details")
        .and_then(Value::as_array)
        .and_then(|details| {
            details.iter().find_map(|detail| {
                if detail.get("@type").and_then(Value::as_str)
                    != Some("type.googleapis.com/google.rpc.RetryInfo")
                {
                    return None;
                }
                detail
                    .get("retryDelay")
                    .and_then(Value::as_str)
                    .and_then(|delay| delay.strip_suffix('s'))
                    .and_then(|delay| delay.parse::<f64>().ok())
            })
        });
    let seconds = retry_after
        .zip(retry_info)
        .map(|(left, right)| left.max(right))
        .or(retry_after)
        .or(retry_info)
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .unwrap_or(0.5 * (1u64 << retry_index) as f64);
    if seconds > 8.0 {
        return None;
    }
    Some(Duration::from_secs_f64(seconds.max(0.1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_url_accepts_sse_and_api_key_queries_without_weakening_origin_validation() {
        for url in [
            "https://aiplatform.googleapis.com/v1/publishers/google/models/gemini:generateContent?key=secret",
            "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1/publishers/google/models/gemini:streamGenerateContent?alt=sse",
        ] { assert!(is_vertex_request_url(url)); }
        for url in [
            "https://aiplatform.googleapis.com.attacker.example/v1?key=secret",
            "http://aiplatform.googleapis.com/v1?key=secret",
            "https://attacker@aiplatform.googleapis.com/v1?key=secret",
            "https://aiplatform.googleapis.com:8443/v1?alt=sse",
            "https://aiplatform.googleapis.com/v1#fragment",
        ] {
            assert!(!is_vertex_request_url(url));
        }
    }

    #[test]
    fn shared_capacity_is_bounded_and_not_quota() {
        let body = r#"{"error":{"status":"RESOURCE_EXHAUSTED","message":"Resource exhausted. Please try again later."}}"#;
        assert!(!vertex_error_has_quota_evidence(Some(body)));
        assert_eq!(
            vertex_capacity_retry_delay(429, &BTreeMap::new(), Some(body), 0),
            Some(Duration::from_millis(500))
        );
        assert_eq!(
            vertex_capacity_retry_delay(429, &BTreeMap::new(), Some(body), 1),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            vertex_capacity_retry_delay(429, &BTreeMap::new(), Some(body), 2),
            None
        );
        assert_eq!(
            vertex_capacity_retry_delay(503, &BTreeMap::new(), Some(body), 0),
            None
        );
        let headers = BTreeMap::from([("Retry-After".into(), "999999".into())]);
        assert_eq!(
            vertex_capacity_retry_delay(429, &headers, Some(body), 0),
            None
        );
    }

    #[test]
    fn quota_and_retry_details_are_distinct() {
        for body in [
            r#"{"error":{"status":"RESOURCE_EXHAUSTED","message":"Quota exceeded for model in region"}}"#,
            r#"{"error":{"status":"RESOURCE_EXHAUSTED","message":"Resource exhausted","details":[{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[]}]}}"#,
        ] {
            assert!(vertex_error_has_quota_evidence(Some(body)));
            assert_eq!(
                vertex_capacity_retry_delay(429, &BTreeMap::new(), Some(body), 0),
                None
            );
        }
        let body = r#"{"error":{"status":"RESOURCE_EXHAUSTED","message":"Resource exhausted","details":[{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"2.5s"}]}}"#;
        assert_eq!(
            vertex_capacity_retry_delay(429, &BTreeMap::new(), Some(body), 0),
            Some(Duration::from_millis(2500))
        );
    }
}
