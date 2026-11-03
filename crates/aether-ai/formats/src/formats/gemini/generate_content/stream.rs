use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

use crate::formats::gemini::generate_content::response::{
    gemini_candidate_grounding, gemini_grounding_citations,
};
use crate::formats::shared::response::{build_generated_tool_call_id, canonicalize_tool_arguments};
use crate::formats::shared::sse::encode_json_sse;
use crate::formats::shared::stream_core::common::*;
use crate::formats::shared::AiSurfaceFinalizeError;

#[derive(Default)]
struct GeminiProviderToolState {
    call_id: String,
    name: String,
    arguments: String,
    thought_signature: String,
    started_emitted: bool,
    partial_arguments: Option<Value>,
    continuing: bool,
    partial_position: usize,
    partial_open_paths: BTreeSet<Vec<ArgumentPathSegment>>,
}

#[derive(Default)]
struct GeminiProviderToolResultState {
    content: String,
    emitted: bool,
}

#[derive(Default)]
pub struct GeminiProviderState {
    terminal_observation_only: bool,
    observed_tool_calls: bool,
    response_id: Option<String>,
    model: Option<String>,
    started: bool,
    finished: bool,
    text_parts: BTreeMap<usize, String>,
    reasoning_parts: BTreeMap<usize, String>,
    reasoning_signatures: BTreeMap<usize, String>,
    content_parts: BTreeMap<usize, CanonicalContentPart>,
    tool_calls: BTreeMap<usize, GeminiProviderToolState>,
    tool_results: BTreeMap<usize, GeminiProviderToolResultState>,
    partial_argument_budget: usize,
    /// Last `groundingMetadata` seen. Gemini resends it cumulatively, so the
    /// newest copy is the complete one; citations are emitted once at finish,
    /// when the answer text they index into is whole.
    grounding: Option<Value>,
}

impl GeminiProviderState {
    pub(crate) fn terminal_observation() -> Self {
        Self {
            terminal_observation_only: true,
            ..Self::default()
        }
    }

    fn identity(&self, report_context: &Value) -> (String, String) {
        resolve_identity(
            self.response_id.as_deref(),
            self.model.as_deref(),
            report_context,
            "resp-local-stream",
        )
    }

    fn ensure_started(&mut self, report_context: &Value, out: &mut Vec<CanonicalStreamFrame>) {
        if self.started {
            return;
        }
        let (id, model) = self.identity(report_context);
        out.push(CanonicalStreamFrame {
            id,
            model,
            event: CanonicalStreamEvent::Start,
        });
        self.started = true;
    }

    fn ensure_partial_calls_complete(&self) -> Result<(), AiSurfaceFinalizeError> {
        if self
            .tool_calls
            .values()
            .any(|state| state.continuing || !state.partial_open_paths.is_empty())
        {
            return Err(AiSurfaceFinalizeError::new(
                "Gemini stream ended during a partial function call",
            ));
        }
        Ok(())
    }

    /// Turn the grounding metadata collected over the stream into citations.
    ///
    /// The offsets Gemini reports index into the finished answer, so this can
    /// only run once the text is complete — hence a single frame just ahead of
    /// `Finish` rather than a delta per chunk.
    fn push_citations_frame(&mut self, id: &str, model: &str, out: &mut Vec<CanonicalStreamFrame>) {
        let Some(grounding) = self.grounding.take() else {
            return;
        };
        let text = self
            .text_parts
            .values()
            .map(String::as_str)
            .collect::<String>();
        let citations = gemini_grounding_citations(&grounding, &text);
        if citations.is_empty() {
            return;
        }
        out.push(CanonicalStreamFrame {
            id: id.to_string(),
            model: model.to_string(),
            event: CanonicalStreamEvent::Citations(citations),
        });
    }

    fn unknown_frame(&self, report_context: &Value, payload: Value) -> CanonicalStreamFrame {
        let (id, model) = self.identity(report_context);
        CanonicalStreamFrame {
            id,
            model,
            event: CanonicalStreamEvent::UnknownEvent(payload),
        }
    }

    pub fn push_line(
        &mut self,
        report_context: &Value,
        line: Vec<u8>,
    ) -> Result<Vec<CanonicalStreamFrame>, AiSurfaceFinalizeError> {
        let Some(value) = decode_json_data_line(&line) else {
            return Ok(Vec::new());
        };
        let Some(raw_event_object) = value.as_object() else {
            return Ok(Vec::new());
        };
        if let Some(id) = raw_event_object.get("responseId").and_then(Value::as_str) {
            self.response_id = Some(id.to_string());
        }
        let event_object = raw_event_object
            .get("response")
            .and_then(Value::as_object)
            .filter(|response| response.contains_key("candidates"))
            .unwrap_or(raw_event_object);
        if let Some(id) = event_object.get("responseId").and_then(Value::as_str) {
            self.response_id = Some(id.to_string());
        }
        if let Some(version) = event_object.get("modelVersion").and_then(Value::as_str) {
            self.model = Some(version.to_string());
        }

        let mut out = Vec::new();
        let Some(candidates) = event_object.get("candidates").and_then(Value::as_array) else {
            out.push(self.unknown_frame(report_context, value.clone()));
            return Ok(out);
        };

        for candidate in candidates {
            let Some(candidate_object) = candidate.as_object() else {
                continue;
            };
            let (response_id, response_model) = self.identity(report_context);
            let terminal_error = gemini_stream_terminal_error_payload(
                candidate_object,
                response_id.as_str(),
                response_model.as_str(),
                event_object.get("usageMetadata"),
            );
            if !self.terminal_observation_only {
                if let Some(grounding) = gemini_candidate_grounding(candidate_object) {
                    self.grounding = Some(grounding.clone());
                }
            }
            let Some(content) = candidate_object.get("content").and_then(Value::as_object) else {
                if let Some(payload) = terminal_error {
                    out.push(self.unknown_frame(report_context, payload));
                    self.finished = true;
                }
                continue;
            };
            let Some(parts) = content.get("parts").and_then(Value::as_array) else {
                if let Some(payload) = terminal_error {
                    out.push(self.unknown_frame(report_context, payload));
                    self.finished = true;
                }
                continue;
            };
            if !parts.is_empty() {
                self.ensure_started(report_context, &mut out);
            }
            let (id, model) = self.identity(report_context);
            let mut chunk_tool_calls = BTreeSet::new();
            for (index, part) in parts.iter().enumerate() {
                let Some(part_object) = part.as_object() else {
                    continue;
                };
                let reasoning_signature = if self.terminal_observation_only {
                    None
                } else {
                    part_object
                        .get("thoughtSignature")
                        .or_else(|| part_object.get("thought_signature"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                };
                if let Some(text) = render_gemini_part_as_text(part_object) {
                    if self.terminal_observation_only {
                        continue;
                    }
                    let is_reasoning = part_object
                        .get("thought")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        || (text.trim().is_empty() && reasoning_signature.is_some());
                    let previous = if is_reasoning {
                        self.reasoning_parts.entry(index).or_default()
                    } else {
                        self.text_parts.entry(index).or_default()
                    };
                    let delta = if text.starts_with(previous.as_str()) {
                        text[previous.len()..].to_string()
                    } else if previous.as_str() == text {
                        String::new()
                    } else {
                        text.to_string()
                    };
                    *previous = text;
                    if !delta.is_empty() {
                        out.push(CanonicalStreamFrame {
                            id: id.clone(),
                            model: model.clone(),
                            event: if is_reasoning {
                                CanonicalStreamEvent::ReasoningDelta(delta)
                            } else {
                                CanonicalStreamEvent::TextDelta(delta)
                            },
                        });
                    }
                    if is_reasoning {
                        if let Some(signature) = reasoning_signature.as_ref() {
                            let previous_signature =
                                self.reasoning_signatures.entry(index).or_default();
                            if previous_signature.as_str() != signature.as_str() {
                                *previous_signature = signature.clone();
                                out.push(CanonicalStreamFrame {
                                    id: id.clone(),
                                    model: model.clone(),
                                    event: CanonicalStreamEvent::ReasoningSignature(
                                        signature.clone(),
                                    ),
                                });
                            }
                        }
                    }
                    continue;
                }
                if let Some(function_response) = part_object
                    .get("functionResponse")
                    .or_else(|| part_object.get("function_response"))
                    .and_then(Value::as_object)
                {
                    if self.terminal_observation_only {
                        continue;
                    }
                    let tool_use_id = function_response
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| build_generated_tool_call_id(index));
                    let name = function_response
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned);
                    let content = gemini_function_response_content(
                        function_response.get("response").unwrap_or(&Value::Null),
                    );
                    let state = self.tool_results.entry(index).or_default();
                    let delta = if !state.emitted {
                        content.clone()
                    } else if content.starts_with(&state.content) {
                        content[state.content.len()..].to_string()
                    } else if state.content == content {
                        String::new()
                    } else {
                        content.clone()
                    };
                    if !delta.is_empty() || !state.emitted {
                        state.emitted = true;
                        state.content.push_str(&delta);
                        out.push(CanonicalStreamFrame {
                            id: id.clone(),
                            model: model.clone(),
                            event: CanonicalStreamEvent::ToolResultDelta {
                                index,
                                tool_use_id,
                                name,
                                content: delta,
                            },
                        });
                    }
                    continue;
                }
                let Some(function_call) =
                    part_object.get("functionCall").and_then(Value::as_object)
                else {
                    if let Some(content_part) = canonical_content_part_from_gemini_part(part_object)
                    {
                        if self.terminal_observation_only {
                            continue;
                        }
                        let should_emit = self
                            .content_parts
                            .get(&index)
                            .map(|existing| existing != &content_part)
                            .unwrap_or(true);
                        if should_emit {
                            self.content_parts.insert(index, content_part.clone());
                            out.push(CanonicalStreamFrame {
                                id: id.clone(),
                                model: model.clone(),
                                event: CanonicalStreamEvent::ContentPart(content_part),
                            });
                        }
                    } else {
                        out.push(
                            self.unknown_frame(report_context, Value::Object(part_object.clone())),
                        );
                    }
                    continue;
                };
                if self.terminal_observation_only {
                    self.observed_tool_calls = true;
                    continue;
                }
                // Complete calls retain arrival-order identity (some adapters repeat
                // synthetic ids). Partial calls instead reuse their open call, keyed
                // by the provider id when available, then unambiguous open
                // name/position matches. Text parts may disappear between chunks.
                let provider_id = function_call
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty());
                let call_name = function_call
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty());
                let by_id = provider_id.and_then(|call_id| {
                    self.tool_calls
                        .iter()
                        .find(|(_, state)| {
                            state.call_id == call_id && state.partial_arguments.is_some()
                        })
                        .map(|(index, _)| *index)
                });
                let open = self
                    .tool_calls
                    .iter()
                    .filter(|(index, state)| {
                        !chunk_tool_calls.contains(*index)
                            && state.continuing
                            && provider_id
                                .is_none_or(|id| state.call_id.is_empty() || state.call_id == id)
                            && call_name
                                .is_none_or(|name| state.name.is_empty() || state.name == name)
                    })
                    .map(|(index, _)| *index)
                    .collect::<Vec<_>>();
                let existing = by_id.or_else(|| match open.as_slice() {
                    [only] => Some(*only),
                    _ => open
                        .iter()
                        .copied()
                        .find(|key| self.tool_calls[key].partial_position == index),
                });
                let partial = function_call.contains_key("partialArgs")
                    || function_call.contains_key("willContinue")
                    || existing.is_some();
                if partial && existing.is_none() && open.len() > 1 {
                    return Err(AiSurfaceFinalizeError::new(
                        "Ambiguous Gemini partial function call identity",
                    ));
                }
                let position = index;
                let index = existing.unwrap_or(self.tool_calls.len());
                if index >= 1024 {
                    return Err(AiSurfaceFinalizeError::new(
                        "Gemini tool call count exceeds limit",
                    ));
                }
                chunk_tool_calls.insert(index);
                let tool_state = self.tool_calls.entry(index).or_default();
                tool_state.call_id = function_call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or(tool_state.call_id.as_str())
                    .to_string();
                tool_state.name = function_call
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(tool_state.name.as_str())
                    .to_string();
                if let Some(signature) = reasoning_signature {
                    tool_state.thought_signature = signature;
                }
                // Canonical argument deltas are append-only JSON text. Buffer
                // JSONPath updates until the call closes so later fields and
                // string fragments cannot invalidate already emitted JSON.
                if partial && !tool_state.arguments.is_empty() {
                    continue;
                }
                if partial {
                    tool_state.partial_position = position;
                    tool_state.continuing = function_call
                        .get("willContinue")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let arguments = tool_state
                        .partial_arguments
                        .get_or_insert_with(|| json!({}));
                    if let Some(args) = function_call.get("args") {
                        charge_partial_argument_budget(
                            &mut self.partial_argument_budget,
                            serde_json::to_vec(args)
                                .map(|bytes| bytes.len())
                                .unwrap_or(usize::MAX),
                        )?;
                        *arguments = args.clone();
                        tool_state.partial_open_paths.clear();
                    }
                    if let Some(parts) = function_call.get("partialArgs").and_then(Value::as_array)
                    {
                        for part in parts {
                            let path = apply_partial_argument(
                                arguments,
                                part,
                                &mut self.partial_argument_budget,
                            )?;
                            if part
                                .get("willContinue")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                            {
                                tool_state.partial_open_paths.insert(path);
                            } else {
                                tool_state.partial_open_paths.remove(&path);
                            }
                        }
                    }
                    if !tool_state.continuing && !tool_state.partial_open_paths.is_empty() {
                        return Err(AiSurfaceFinalizeError::new(
                            "Gemini function call closed before its argument",
                        ));
                    }
                    if tool_state.continuing {
                        continue;
                    }
                }
                // Partial calls can announce their name or id in a later chunk.
                // Emit identity only when the call closes, alongside its buffered
                // arguments, so every downstream format receives the final identity.
                if partial && tool_state.name.is_empty() {
                    return Err(AiSurfaceFinalizeError::new(
                        "Gemini partial function call completed without a name",
                    ));
                }
                if !tool_state.thought_signature.is_empty() {
                    out.push(CanonicalStreamFrame {
                        id: id.clone(),
                        model: model.clone(),
                        event: CanonicalStreamEvent::ToolCallSignature {
                            index,
                            signature: tool_state.thought_signature.clone(),
                        },
                    });
                }
                if !tool_state.started_emitted {
                    out.push(CanonicalStreamFrame {
                        id: id.clone(),
                        model: model.clone(),
                        event: CanonicalStreamEvent::ToolCallStart {
                            index,
                            call_id: if tool_state.call_id.is_empty() {
                                build_generated_tool_call_id(index)
                            } else {
                                tool_state.call_id.clone()
                            },
                            name: if tool_state.name.is_empty() {
                                "unknown".to_string()
                            } else {
                                tool_state.name.clone()
                            },
                        },
                    });
                    tool_state.started_emitted = true;
                }
                let arguments = canonicalize_tool_arguments(
                    tool_state
                        .partial_arguments
                        .clone()
                        .or_else(|| function_call.get("args").cloned()),
                );
                let delta = if arguments.starts_with(&tool_state.arguments) {
                    arguments[tool_state.arguments.len()..].to_string()
                } else if tool_state.arguments == arguments {
                    String::new()
                } else {
                    arguments.clone()
                };
                tool_state.arguments = arguments;
                if !delta.is_empty() {
                    out.push(CanonicalStreamFrame {
                        id: id.clone(),
                        model: model.clone(),
                        event: CanonicalStreamEvent::ToolCallArgumentsDelta {
                            index,
                            arguments: delta,
                        },
                    });
                }
            }
            if let Some(payload) = terminal_error {
                out.push(self.unknown_frame(report_context, payload));
                self.finished = true;
                continue;
            }
            if let Some(finish_reason) =
                candidate_object.get("finishReason").and_then(Value::as_str)
            {
                let has_tool_calls = self.observed_tool_calls || !self.tool_calls.is_empty();
                let mut finish_reason =
                    normalize_openai_finish_reason(map_gemini_stream_finish_reason(finish_reason));
                if has_tool_calls && finish_reason.as_deref().is_none_or(|value| value == "stop") {
                    finish_reason = Some("tool_calls".to_string());
                }
                self.ensure_partial_calls_complete()?;
                self.push_citations_frame(&id, &model, &mut out);
                out.push(CanonicalStreamFrame {
                    id,
                    model,
                    event: CanonicalStreamEvent::Finish {
                        finish_reason,
                        usage: canonical_usage_from_gemini_usage(event_object.get("usageMetadata")),
                    },
                });
                self.finished = true;
            }
        }

        Ok(out)
    }

    pub fn finish(
        &mut self,
        report_context: &Value,
    ) -> Result<Vec<CanonicalStreamFrame>, AiSurfaceFinalizeError> {
        if !self.started || self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        let (id, model) = self.identity(report_context);
        let mut out = Vec::new();
        self.ensure_partial_calls_complete()?;
        self.push_citations_frame(&id, &model, &mut out);
        out.push(CanonicalStreamFrame {
            id,
            model,
            event: CanonicalStreamEvent::Finish {
                finish_reason: None,
                usage: None,
            },
        });
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ArgumentPathSegment {
    Key(String),
    Index(usize),
}

/// Function arguments identify a single value, not a JSONPath query. Accept
/// member and array selectors, including quoted member names; reject wildcards,
/// slices and recursive selectors rather than guessing their meaning.
fn partial_argument_path(path: &str) -> Option<Vec<ArgumentPathSegment>> {
    let mut remaining = path.strip_prefix('$')?;
    let mut segments = Vec::new();
    while !remaining.is_empty() {
        if segments.len() >= 128 {
            return None;
        }
        if let Some(rest) = remaining.strip_prefix('.') {
            let end = rest.find(['.', '[']).unwrap_or(rest.len());
            let key = &rest[..end];
            if key.is_empty() || !key.chars().all(|c| c == '_' || c.is_alphanumeric()) {
                return None;
            }
            segments.push(ArgumentPathSegment::Key(key.to_string()));
            remaining = &rest[end..];
        } else if let Some(rest) = remaining.strip_prefix('[') {
            if rest.starts_with(['\"', '\'']) {
                let quote = rest.chars().next()?;
                let mut escaped = false;
                let mut end = None;
                for (offset, c) in rest.char_indices().skip(1) {
                    if escaped {
                        escaped = false;
                        continue;
                    }
                    if c == '\\' {
                        escaped = true;
                        continue;
                    }
                    if c == quote {
                        end = Some(offset);
                        break;
                    }
                }
                let end = end?;
                let raw = &rest[1..end];
                // JSONPath single quoted names use the same escapes, with
                // \' instead of \". Convert only those quoting differences.
                let key: String = if quote == '\"' {
                    serde_json::from_str(&rest[..=end]).ok()?
                } else {
                    let mut json = String::from("\"");
                    let mut chars = raw.chars();
                    while let Some(c) = chars.next() {
                        match c {
                            '\\' => match chars.next()? {
                                '\'' => json.push('\''),
                                other => {
                                    json.push('\\');
                                    json.push(other);
                                }
                            },
                            '\"' => json.push_str("\\\""),
                            other => json.push(other),
                        }
                    }
                    json.push('\"');
                    serde_json::from_str(&json).ok()?
                };
                remaining = rest[end + 1..].strip_prefix(']')?;
                segments.push(ArgumentPathSegment::Key(key));
            } else {
                let end = rest.find(']')?;
                let raw = &rest[..end];
                if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let index: usize = raw.parse().ok()?;
                // Avoid unbounded allocation on malformed upstream paths.
                if index > 65535 {
                    return None;
                }
                segments.push(ArgumentPathSegment::Index(index));
                remaining = &rest[end + 1..];
            }
        } else {
            return None;
        }
    }
    Some(segments)
}

fn charge_partial_argument_budget(
    budget: &mut usize,
    amount: usize,
) -> Result<(), AiSurfaceFinalizeError> {
    *budget = budget.saturating_add(amount);
    if *budget > 8 * 1024 * 1024 {
        return Err(AiSurfaceFinalizeError::new(
            "Gemini partial arguments exceed buffer limit",
        ));
    }
    Ok(())
}

fn apply_partial_argument(
    arguments: &mut Value,
    part: &Value,
    budget: &mut usize,
) -> Result<Vec<ArgumentPathSegment>, AiSurfaceFinalizeError> {
    let Some(path) = part
        .get("jsonPath")
        .and_then(Value::as_str)
        .and_then(partial_argument_path)
    else {
        return Err(AiSurfaceFinalizeError::new(
            "Invalid Gemini partial argument path or value",
        ));
    };
    let mut deltas = ["stringValue", "numberValue", "boolValue", "nullValue"]
        .iter()
        .filter_map(|key| part.get(*key).map(|value| (*key, value)));
    let Some((kind, delta)) = deltas.next() else {
        return Err(AiSurfaceFinalizeError::new(
            "Missing Gemini partial argument scalar",
        ));
    };
    if deltas.next().is_some()
        || !match kind {
            "stringValue" => delta.is_string(),
            "numberValue" => delta.is_number(),
            "boolValue" => delta.is_boolean(),
            "nullValue" => delta.is_null(),
            _ => false,
        }
    {
        return Err(AiSurfaceFinalizeError::new(
            "Invalid Gemini partial argument scalar union",
        ));
    }
    charge_partial_argument_budget(
        budget,
        serde_json::to_vec(part)
            .map(|bytes| bytes.len())
            .unwrap_or(usize::MAX),
    )?;
    let mut target = arguments;
    for segment in path.clone() {
        match segment {
            ArgumentPathSegment::Key(key) => {
                if target.is_null() {
                    *target = json!({});
                }
                let Some(object) = target.as_object_mut() else {
                    return Err(AiSurfaceFinalizeError::new(
                        "Invalid Gemini partial argument path or value",
                    ));
                };
                target = object.entry(key).or_insert(Value::Null);
            }
            ArgumentPathSegment::Index(index) => {
                if target.is_null() {
                    *target = json!([]);
                }
                let Some(array) = target.as_array_mut() else {
                    return Err(AiSurfaceFinalizeError::new(
                        "Invalid Gemini partial argument path or value",
                    ));
                };
                if array.len() <= index {
                    charge_partial_argument_budget(
                        budget,
                        (index + 1 - array.len()) * std::mem::size_of::<Value>(),
                    )?;
                    array.resize(index + 1, Value::Null);
                }
                target = &mut array[index];
            }
        }
    }
    match (target, delta) {
        (Value::String(existing), Value::String(delta)) => existing.push_str(delta),
        (target, delta) => *target = delta.clone(),
    }
    Ok(path)
}

fn gemini_stream_terminal_error_payload(
    candidate: &Map<String, Value>,
    response_id: &str,
    model: &str,
    usage_metadata: Option<&Value>,
) -> Option<Value> {
    let finish_reason = candidate
        .get("finishReason")
        .or_else(|| candidate.get("finish_reason"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            matches!(
                *value,
                "MALFORMED_FUNCTION_CALL"
                    | "UNEXPECTED_TOOL_CALL"
                    | "TOO_MANY_TOOL_CALLS"
                    | "MISSING_THOUGHT_SIGNATURE"
                    | "MALFORMED_RESPONSE"
            )
        })?;
    let message = candidate
        .get("finishMessage")
        .or_else(|| candidate.get("finish_message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("Gemini stream ended with {finish_reason}"));

    let mut response = json!({
        "id": response_id,
        "object": "response",
        "model": model,
        "status": "failed",
        "error": {
            "type": "upstream_gemini_finish_error",
            "code": finish_reason,
            "message": message,
            "upstream_status": 200
        }
    });
    if let Some(usage) = canonical_usage_from_gemini_usage(usage_metadata)
        .map(|usage| openai_responses_usage_from_usage(&usage))
    {
        response["usage"] = usage;
    }

    Some(json!({
        "type": "response.failed",
        "response": response
    }))
}

fn map_gemini_stream_finish_reason(value: &str) -> Option<&str> {
    match value {
        "STOP" => Some("stop"),
        "MAX_TOKENS" => Some("length"),
        "SAFETY"
        | "RECITATION"
        | "LANGUAGE"
        | "BLOCKLIST"
        | "PROHIBITED_CONTENT"
        | "SPII"
        | "IMAGE_SAFETY"
        | "IMAGE_PROHIBITED_CONTENT"
        | "IMAGE_RECITATION" => Some("content_filter"),
        other => Some(other),
    }
}

#[derive(Default)]
struct GeminiClientToolState {
    call_id: String,
    name: String,
    arguments: String,
    thought_signature: String,
    emitted: bool,
}

#[derive(Default)]
pub struct GeminiClientEmitter {
    response_id: Option<String>,
    model: Option<String>,
    finished: bool,
    tool_calls: BTreeMap<usize, GeminiClientToolState>,
}

impl GeminiClientEmitter {
    fn update_identity(&mut self, frame: &CanonicalStreamFrame) {
        self.response_id = Some(frame.id.clone());
        self.model = Some(frame.model.clone());
    }

    fn emit_candidate(
        &self,
        parts: Vec<Value>,
        finish_reason: Option<&str>,
        usage: Option<CanonicalUsage>,
    ) -> Result<Vec<u8>, AiSurfaceFinalizeError> {
        let mut candidate = Map::new();
        candidate.insert(
            "content".to_string(),
            json!({
                "role": "model",
                "parts": parts,
            }),
        );
        candidate.insert("index".to_string(), Value::from(0_u64));
        if let Some(finish_reason) = finish_reason {
            candidate.insert(
                "finishReason".to_string(),
                Value::String(map_openai_finish_reason_to_gemini(Some(finish_reason)).to_string()),
            );
        }
        let mut response = Map::new();
        response.insert(
            "responseId".to_string(),
            Value::String(
                self.response_id
                    .clone()
                    .unwrap_or_else(|| "resp-local-stream".to_string()),
            ),
        );
        response.insert(
            "modelVersion".to_string(),
            Value::String(self.model.clone().unwrap_or_else(|| "unknown".to_string())),
        );
        response.insert(
            "candidates".to_string(),
            Value::Array(vec![Value::Object(candidate)]),
        );
        if let Some(usage) = usage {
            response.insert(
                "usageMetadata".to_string(),
                gemini_usage_metadata_from_usage(&usage),
            );
        }
        encode_json_sse(None, &Value::Object(response))
    }

    fn flush_pending_tool_calls(&mut self) -> Result<Vec<u8>, AiSurfaceFinalizeError> {
        let mut out = Vec::new();
        let mut pending = Vec::new();
        for (index, tool_call) in &mut self.tool_calls {
            if tool_call.emitted {
                continue;
            }
            let args_value = parse_json_arguments_value(&tool_call.arguments)
                .unwrap_or_else(|| Value::Object(Map::new()));
            tool_call.emitted = true;
            let mut part = json!({
                "functionCall": {
                    "id": if tool_call.call_id.is_empty() {
                        build_generated_tool_call_id(*index)
                    } else {
                        tool_call.call_id.clone()
                    },
                    "name": if tool_call.name.is_empty() {
                        "unknown".to_string()
                    } else {
                        tool_call.name.clone()
                    },
                    "args": args_value,
                }
            });
            if !tool_call.thought_signature.is_empty() {
                part["thoughtSignature"] = Value::String(tool_call.thought_signature.clone());
            }
            pending.push(part);
        }
        for part in pending {
            out.extend(self.emit_candidate(vec![part], None, None)?);
        }
        Ok(out)
    }

    pub fn emit(&mut self, frame: CanonicalStreamFrame) -> Result<Vec<u8>, AiSurfaceFinalizeError> {
        self.update_identity(&frame);
        match frame.event {
            CanonicalStreamEvent::Start => Ok(Vec::new()),
            CanonicalStreamEvent::TextDelta(text) => {
                self.emit_candidate(vec![json!({ "text": text })], None, None)
            }
            CanonicalStreamEvent::ReasoningDelta(text) => {
                self.emit_candidate(vec![json!({ "text": text, "thought": true })], None, None)
            }
            CanonicalStreamEvent::ReasoningSignature(signature) => self.emit_candidate(
                vec![json!({
                    "text": "",
                    "thought": true,
                    "thoughtSignature": signature,
                })],
                None,
                None,
            ),
            CanonicalStreamEvent::ContentPart(part) => self.emit_candidate(
                vec![gemini_part_from_canonical_content_part(part)],
                None,
                None,
            ),
            CanonicalStreamEvent::ImageGenerationCall { item, .. } => {
                let Some(part) = content_part_from_openai_image_generation_item(&item) else {
                    return Ok(Vec::new());
                };
                self.emit_candidate(
                    vec![gemini_part_from_canonical_content_part(part)],
                    None,
                    None,
                )
            }
            CanonicalStreamEvent::OpenAiResponsesOutputItem { .. } => Err(
                AiSurfaceFinalizeError::new(
                    "OpenAI Responses output items cannot be converted losslessly to Gemini GenerateContent",
                ),
            ),
            CanonicalStreamEvent::ToolCallStart {
                index,
                call_id,
                name,
            } => {
                let state = self.tool_calls.entry(index).or_default();
                state.call_id = call_id;
                state.name = name;
                Ok(Vec::new())
            }
            CanonicalStreamEvent::ToolCallSignature { index, signature } => {
                self.tool_calls.entry(index).or_default().thought_signature = signature;
                Ok(Vec::new())
            }
            CanonicalStreamEvent::ToolCallArgumentsDelta { index, arguments } => {
                let emitted_part = {
                    let state = self.tool_calls.entry(index).or_default();
                    state.arguments.push_str(&arguments);
                    if state.emitted {
                        None
                    } else {
                        let args_value = parse_json_arguments_value(&state.arguments);
                        args_value.map(|args_value| {
                            state.emitted = true;
                            let mut part = json!({
                                "functionCall": {
                                    "id": if state.call_id.is_empty() {
                                        build_generated_tool_call_id(index)
                                    } else {
                                        state.call_id.clone()
                                    },
                                    "name": if state.name.is_empty() {
                                        "unknown".to_string()
                                    } else {
                                        state.name.clone()
                                    },
                                    "args": args_value,
                                }
                            });
                            if !state.thought_signature.is_empty() {
                                part["thoughtSignature"] =
                                    Value::String(state.thought_signature.clone());
                            }
                            part
                        })
                    }
                };
                let Some(part) = emitted_part else {
                    return Ok(Vec::new());
                };
                self.emit_candidate(vec![part], None, None)
            }
            CanonicalStreamEvent::ToolResultDelta {
                tool_use_id,
                name,
                content,
                ..
            } => self.emit_candidate(
                vec![gemini_function_response_part(tool_use_id, name, content)],
                None,
                None,
            ),
            // Only Gemini produces citations today, and a Gemini-to-Gemini
            // stream keeps its own `groundingMetadata` on the passthrough path.
            CanonicalStreamEvent::Citations(_) => Ok(Vec::new()),
            CanonicalStreamEvent::UnknownEvent(_) => Ok(Vec::new()),
            CanonicalStreamEvent::Finish {
                finish_reason,
                usage,
            } => {
                if self.finished {
                    return Ok(Vec::new());
                }
                let mut out = self.flush_pending_tool_calls()?;
                out.extend(self.emit_candidate(vec![], finish_reason.as_deref(), usage)?);
                self.finished = true;
                Ok(out)
            }
            CanonicalStreamEvent::ReasoningSummaryDone => Ok(Vec::new()),
        }
    }

    pub fn finish(&mut self) -> Result<Vec<u8>, AiSurfaceFinalizeError> {
        if self.finished {
            return Ok(Vec::new());
        }
        let out = self.flush_pending_tool_calls()?;
        self.finished = true;
        Ok(out)
    }
}

fn gemini_function_response_part(
    tool_use_id: String,
    name: Option<String>,
    content: String,
) -> Value {
    json!({
        "functionResponse": {
            "id": tool_use_id,
            "name": name
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "unknown".to_string()),
            "response": gemini_function_response_value(&content),
        }
    })
}

fn gemini_function_response_value(content: &str) -> Value {
    if content.trim().is_empty() {
        return Value::Object(Map::new());
    }
    match serde_json::from_str::<Value>(content) {
        Ok(Value::Object(map)) => Value::Object(map),
        Ok(value) => json!({ "output": value }),
        Err(_) => json!({ "output": content }),
    }
}

fn gemini_function_response_content(response: &Value) -> String {
    match response {
        Value::Object(object) => object
            .get("result")
            .cloned()
            .unwrap_or_else(|| Value::Object(object.clone()))
            .to_string(),
        Value::Null => String::new(),
        value => value.to_string(),
    }
}

fn render_gemini_part_as_text(part: &Map<String, Value>) -> Option<String> {
    if let Some(text) = part.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(code) = part.get("executableCode").and_then(Value::as_object) {
        let language = code
            .get("language")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let source = code.get("code").and_then(Value::as_str).unwrap_or_default();
        return Some(format!("```{language}\n{source}\n```"));
    }
    if let Some(result) = part.get("codeExecutionResult").and_then(Value::as_object) {
        let output = result
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Some(format!("```output\n{output}\n```"));
    }
    None
}

fn canonical_content_part_from_gemini_part(
    part: &Map<String, Value>,
) -> Option<CanonicalContentPart> {
    if let Some(image_url) = extract_gemini_image_url(part) {
        return Some(CanonicalContentPart::ImageUrl(image_url));
    }
    if let Some(inline_data) = part
        .get("inlineData")
        .or_else(|| part.get("inline_data"))
        .and_then(Value::as_object)
    {
        let mime_type = inline_data
            .get("mimeType")
            .or_else(|| inline_data.get("mime_type"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let data = inline_data
            .get("data")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        if let Some(format) = mime_type.strip_prefix("audio/") {
            return Some(CanonicalContentPart::Audio {
                data: data.to_string(),
                format: format.to_string(),
            });
        }
        return Some(CanonicalContentPart::File {
            file_data: Some(format!("data:{mime_type};base64,{data}")),
            reference: None,
            mime_type: Some(mime_type.to_string()),
            filename: None,
        });
    }
    let file_data = part
        .get("fileData")
        .or_else(|| part.get("file_data"))
        .and_then(Value::as_object)?;
    let reference = file_data
        .get("fileUri")
        .or_else(|| file_data.get("file_uri"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(CanonicalContentPart::File {
        file_data: None,
        reference: Some(reference.to_string()),
        mime_type: file_data
            .get("mimeType")
            .or_else(|| file_data.get("mime_type"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        filename: None,
    })
}

fn gemini_part_from_canonical_content_part(part: CanonicalContentPart) -> Value {
    match part {
        CanonicalContentPart::ImageUrl(url) => {
            if let Some((mime_type, data)) = parse_data_url(url.as_str()) {
                json!({
                    "inlineData": {
                        "mimeType": mime_type,
                        "data": data,
                    }
                })
            } else {
                json!({
                    "fileData": {
                        "fileUri": url.clone(),
                        "mimeType": guess_media_type_from_reference(url.as_str(), "image/jpeg"),
                    }
                })
            }
        }
        CanonicalContentPart::File {
            file_data,
            reference,
            mime_type,
            ..
        } => {
            if let Some(file_data) = file_data {
                if let Some((mime_type, data)) = parse_data_url(file_data.as_str()) {
                    json!({
                        "inlineData": {
                            "mimeType": mime_type,
                            "data": data,
                        }
                    })
                } else {
                    json!({ "text": "[File]" })
                }
            } else if let Some(reference) = reference {
                json!({
                    "fileData": {
                        "fileUri": reference.clone(),
                        "mimeType": mime_type.unwrap_or_else(|| {
                            guess_media_type_from_reference(reference.as_str(), "application/octet-stream")
                        }),
                    }
                })
            } else {
                json!({ "text": "[File]" })
            }
        }
        CanonicalContentPart::Audio { data, format } => json!({
            "inlineData": {
                "mimeType": format!("audio/{format}"),
                "data": data,
            }
        }),
    }
}

fn extract_gemini_image_url(part: &Map<String, Value>) -> Option<String> {
    let inline_data = part
        .get("inlineData")
        .or_else(|| part.get("inline_data"))
        .and_then(Value::as_object)?;
    if !inline_data
        .get("mimeType")
        .or_else(|| inline_data.get("mime_type"))
        .and_then(Value::as_str)
        .is_some_and(|value| value.starts_with("image/"))
    {
        return None;
    }
    if let Some(data) = inline_data
        .get("data")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let mime_type = inline_data
            .get("mimeType")
            .or_else(|| inline_data.get("mime_type"))
            .and_then(Value::as_str)
            .unwrap_or("image/jpeg");
        return Some(format!("data:{mime_type};base64,{data}"));
    }
    let file_data = part
        .get("fileData")
        .or_else(|| part.get("file_data"))
        .and_then(Value::as_object)?;
    if !file_data
        .get("mimeType")
        .or_else(|| file_data.get("mime_type"))
        .and_then(Value::as_str)
        .is_some_and(|value| value.starts_with("image/"))
    {
        return None;
    }
    file_data
        .get("fileUri")
        .or_else(|| file_data.get("file_uri"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn parse_data_url(value: &str) -> Option<(String, String)> {
    let rest = value.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime_type = meta.strip_suffix(";base64")?;
    if mime_type.trim().is_empty() || data.trim().is_empty() {
        return None;
    }
    Some((mime_type.to_string(), data.to_string()))
}

fn guess_media_type_from_reference(reference: &str, default_mime: &str) -> String {
    let normalized = reference
        .split('?')
        .next()
        .unwrap_or(reference)
        .to_ascii_lowercase();
    if normalized.ends_with(".png") {
        "image/png".to_string()
    } else if normalized.ends_with(".gif") {
        "image/gif".to_string()
    } else if normalized.ends_with(".webp") {
        "image/webp".to_string()
    } else if normalized.ends_with(".jpg") || normalized.ends_with(".jpeg") {
        "image/jpeg".to_string()
    } else if normalized.ends_with(".pdf") {
        "application/pdf".to_string()
    } else {
        default_mime.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn data_line(value: Value) -> Vec<u8> {
        format!("data: {}\n", value).into_bytes()
    }

    fn partial_chunk(parts: Vec<Value>) -> Vec<u8> {
        data_line(json!({"candidates": [{"content": {"parts": parts}}]}))
    }

    fn argument_deltas(frames: &[CanonicalStreamFrame]) -> Vec<(usize, Value)> {
        frames
            .iter()
            .filter_map(|frame| match &frame.event {
                CanonicalStreamEvent::ToolCallArgumentsDelta { index, arguments } => {
                    Some((*index, serde_json::from_str(arguments).unwrap()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn vertex_partial_args_parallel_calls_keep_ids_signatures_and_nested_values() {
        let mut state = GeminiProviderState::default();
        let first = state.push_line(&json!({}), partial_chunk(vec![
            json!({"thoughtSignature":"sig-a", "functionCall": {"id":"a", "name":"search", "willContinue":true,
                "partialArgs":[{"jsonPath":"$.items[0]['text.value']", "stringValue":"hel", "willContinue":true}]}}),
            json!({"functionCall": {"id":"b", "name":"search", "willContinue":true,
                "partialArgs":[{"jsonPath":"$.ok", "boolValue":true}]}}),
        ])).unwrap();
        assert!(argument_deltas(&first).is_empty());
        assert_eq!(
            first
                .iter()
                .filter(|f| matches!(f.event, CanonicalStreamEvent::ToolCallStart { .. }))
                .count(),
            0
        );
        assert!(!first
            .iter()
            .any(|f| matches!(&f.event, CanonicalStreamEvent::ToolCallSignature { .. })));
        // Reordered parts are associated by id, even when both tools have the same name.
        let last = state.push_line(&json!({}), partial_chunk(vec![
            json!({"functionCall":{"id":"b", "willContinue":false, "partialArgs":[{"jsonPath":"$.count", "numberValue":2}]}}),
            json!({"functionCall":{"id":"a", "willContinue":false, "partialArgs":[
                {"jsonPath":"$['items'][0]['text.value']", "stringValue":"lo"},
                {"jsonPath":"$.items[1]", "nullValue":null}]}}),
        ])).unwrap();
        assert_eq!(
            argument_deltas(&last),
            vec![
                (1, json!({"ok":true,"count":2})),
                (0, json!({"items":[{"text.value":"hello"},null]}))
            ]
        );
        assert_eq!(
            last.iter()
                .filter(|f| matches!(f.event, CanonicalStreamEvent::ToolCallStart { .. }))
                .count(),
            2
        );
        assert!(last.iter().any(|f| matches!(&f.event, CanonicalStreamEvent::ToolCallSignature {index:0, signature} if signature == "sig-a")));
        assert!(state.finish(&json!({})).is_ok());
    }

    #[test]
    fn vertex_partial_args_idless_parallel_calls_and_full_args_completion() {
        let mut state = GeminiProviderState::default();
        state.push_line(&json!({}), partial_chunk(vec![
            json!({"functionCall":{"name":"f", "willContinue":true, "partialArgs":[{"jsonPath":"$.x", "stringValue":"a", "willContinue":true}]}}),
            json!({"functionCall":{"name":"f", "willContinue":true, "partialArgs":[{"jsonPath":"$.y", "numberValue":1}]}}),
        ])).unwrap();
        let frames = state
            .push_line(
                &json!({}),
                partial_chunk(vec![
                    json!({"functionCall":{"args":{"x":"authoritative"}}}),
                    json!({"functionCall":{"partialArgs":[{"jsonPath":"$.y", "numberValue":2}]}}),
                ]),
            )
            .unwrap();
        assert_eq!(
            argument_deltas(&frames),
            vec![(0, json!({"x":"authoritative"})), (1, json!({"y":2}))]
        );
        assert_eq!(state.tool_calls.len(), 2);
        assert_eq!(state.tool_calls[&0].call_id, "");
        assert!(state.finish(&json!({})).is_ok());
    }

    #[test]
    fn vertex_partial_args_incomplete_stream_is_error_and_never_emits_arguments() {
        let mut state = GeminiProviderState::default();
        let frames = state.push_line(&json!({}), partial_chunk(vec![json!({"functionCall":{
            "name":"f", "willContinue":true, "partialArgs":[{"jsonPath":"$.x", "stringValue":"unfinished", "willContinue":true}]
        }})])).unwrap();
        assert!(argument_deltas(&frames).is_empty());
        assert!(state.finish(&json!({})).is_err());
        assert!(state.tool_calls[&0].arguments.is_empty());
    }

    #[test]
    fn vertex_partial_args_extends_initial_args_and_deduplicates_completed_id() {
        let mut state = GeminiProviderState::default();
        state
            .push_line(
                &json!({}),
                partial_chunk(vec![json!({"functionCall":{
                    "id":"a", "name":"f", "args":{"x":"a"}, "willContinue":true
                }})]),
            )
            .unwrap();
        let final_chunk = partial_chunk(vec![json!({"functionCall":{
            "id":"a", "partialArgs":[{"jsonPath":"$.x", "stringValue":"b"}]
        }})]);
        let frames = state.push_line(&json!({}), final_chunk.clone()).unwrap();
        assert_eq!(argument_deltas(&frames), vec![(0, json!({"x":"ab"}))]);
        assert!(state.push_line(&json!({}), final_chunk).unwrap().is_empty());
        assert_eq!(state.tool_calls.len(), 1);
    }

    #[test]
    fn vertex_partial_args_tracks_unique_call_when_text_part_disappears() {
        let mut state = GeminiProviderState::default();
        state.push_line(&json!({}), partial_chunk(vec![json!({"text":"thinking"}), json!({"functionCall":{
            "name":"f", "willContinue":true, "partialArgs":[{"jsonPath":"$.x", "stringValue":"a", "willContinue":true}]
        }})])).unwrap();
        let last = state
            .push_line(
                &json!({}),
                partial_chunk(vec![json!({"functionCall":{
                    "willContinue":false, "partialArgs":[{"jsonPath":"$.x", "stringValue":"b"}]
                }})]),
            )
            .unwrap();
        assert_eq!(state.tool_calls.len(), 1);
        assert_eq!(argument_deltas(&last), vec![(0, json!({"x":"ab"}))]);
        assert!(state.finish(&json!({})).is_ok());
    }

    #[test]
    fn vertex_partial_args_rejects_ambiguous_idless_continuation() {
        let mut state = GeminiProviderState::default();
        state
            .push_line(
                &json!({}),
                partial_chunk(vec![
                    json!({"text":"a"}),
                    json!({"text":"b"}),
                    json!({"functionCall":{"name":"f", "willContinue":true, "partialArgs":[]}}),
                    json!({"functionCall":{"name":"f", "willContinue":true, "partialArgs":[]}}),
                ]),
            )
            .unwrap();
        assert!(state
            .push_line(
                &json!({}),
                partial_chunk(vec![json!({"functionCall":{
                    "partialArgs":[{"jsonPath":"$.x", "numberValue":1}]
                }})])
            )
            .is_err());
    }

    #[test]
    fn vertex_partial_args_converts_to_chat_responses_claude_and_gemini() {
        use crate::formats::shared::stream_core::format_matrix::StreamingStandardFormatMatrix;
        let expected = json!({"nested":[{"text":"hello"}]});
        for client in [
            "openai:chat",
            "openai:responses",
            "claude:messages",
            "gemini:generate_content",
        ] {
            let context = json!({"provider_api_format":"gemini:generate_content", "client_api_format":client, "mapped_model":"test"});
            let mut matrix = StreamingStandardFormatMatrix::default();
            let mut output = matrix.transform_line(&context, partial_chunk(vec![json!({
                "thoughtSignature":"sig-roundtrip", "functionCall":{"willContinue":true,
                "partialArgs":[{"jsonPath":"$.nested[0].text", "stringValue":"hel", "willContinue":true}]}
            })])).unwrap();
            output.extend(matrix.transform_line(&context, data_line(json!({"candidates":[{"finishReason":"STOP", "content":{"parts":[{
                "functionCall":{"id":"call_vertex", "name":"f", "willContinue":false,
                    "partialArgs":[{"jsonPath":"$.nested[0].text", "stringValue":"lo"}]}
            }]}}]}))).unwrap());
            output.extend(matrix.finish(&context).unwrap());
            assert!(
                !String::from_utf8_lossy(&output).contains("unknown"),
                "{client}"
            );
            assert!(
                String::from_utf8_lossy(&output).contains("call_vertex"),
                "{client}"
            );
            let events = String::from_utf8(output)
                .unwrap()
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .collect::<Vec<_>>();
            let args = match client {
                "openai:chat" => events.iter().find_map(|event| {
                    event
                        .pointer("/choices/0/delta/tool_calls/0/function/arguments")
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                }),
                "openai:responses" => events
                    .iter()
                    .find(|event| event["type"] == "response.function_call_arguments.done")
                    .and_then(|event| event["arguments"].as_str()),
                "claude:messages" => events
                    .iter()
                    .find_map(|event| event.pointer("/delta/partial_json").and_then(Value::as_str)),
                _ => None,
            };
            if client == "gemini:generate_content" {
                let part = events
                    .iter()
                    .filter_map(|event| event.pointer("/candidates/0/content/parts/0"))
                    .find(|part| part.get("functionCall").is_some())
                    .expect("Gemini tool call");
                assert_eq!(part["functionCall"]["args"], expected);
                assert_eq!(part["thoughtSignature"], "sig-roundtrip");
                assert_eq!(part["functionCall"]["id"], "call_vertex");
            } else {
                assert_eq!(
                    serde_json::from_str::<Value>(args.expect(client)).unwrap(),
                    expected,
                    "{client}"
                );
            }
        }
    }

    #[test]
    fn vertex_partial_args_waits_for_late_name_and_id() {
        let mut state = GeminiProviderState::default();
        let first = state.push_line(&json!({}), partial_chunk(vec![json!({
            "thoughtSignature":"late-signature", "functionCall":{"willContinue":true,
            "partialArgs":[{"jsonPath":"$.text", "stringValue":"first", "willContinue":true}]}
        })])).unwrap();
        assert!(!first.iter().any(|frame| matches!(
            frame.event,
            CanonicalStreamEvent::ToolCallStart { .. }
                | CanonicalStreamEvent::ToolCallSignature { .. }
                | CanonicalStreamEvent::ToolCallArgumentsDelta { .. }
        )));
        let middle = state.push_line(&json!({}), partial_chunk(vec![json!({"functionCall":{
            "name":"real_function", "id":"real_id", "willContinue":true,
            "partialArgs":[{"jsonPath":"$.text", "stringValue":" second", "willContinue":true}]
        }})])).unwrap();
        assert!(middle.is_empty());
        let last = state
            .push_line(
                &json!({}),
                partial_chunk(vec![json!({"functionCall":{
                    "partialArgs":[{"jsonPath":"$.text", "stringValue":" third"}]
                }})]),
            )
            .unwrap();
        assert!(last.iter().any(|frame| matches!(&frame.event,
            CanonicalStreamEvent::ToolCallStart {index:0, call_id, name}
                if call_id == "real_id" && name == "real_function")));
        assert!(last.iter().any(|frame| matches!(&frame.event,
            CanonicalStreamEvent::ToolCallSignature {index:0, signature} if signature == "late-signature")));
        assert_eq!(
            argument_deltas(&last),
            vec![(0, json!({"text":"first second third"}))]
        );
        assert_eq!(state.tool_calls.len(), 1);
        assert_eq!(state.tool_calls[&0].call_id, "real_id");
    }

    #[test]
    fn vertex_partial_args_strictly_validates_scalar_union() {
        for part in [
            json!({"jsonPath":"$.x", "stringValue":1}),
            json!({"jsonPath":"$.x", "numberValue":"1"}),
            json!({"jsonPath":"$.x", "boolValue":null}),
            json!({"jsonPath":"$.x", "nullValue":false}),
            json!({"jsonPath":"$.x", "stringValue":"a", "numberValue":1}),
            json!({"jsonPath":"$.x"}),
        ] {
            let mut args = json!({});
            assert!(
                apply_partial_argument(&mut args, &part, &mut 0).is_err(),
                "{part}"
            );
            assert_eq!(args, json!({}));
        }
        let mut args = json!({});
        let mut budget = 0;
        for _ in 0..2048 {
            apply_partial_argument(
                &mut args,
                &json!({"jsonPath":"$.long", "stringValue":"fragment"}),
                &mut budget,
            )
            .unwrap();
        }
        assert_eq!(args["long"].as_str().unwrap(), "fragment".repeat(2048));
        let mut state = GeminiProviderState::default();
        assert!(state
            .push_line(
                &json!({}),
                partial_chunk(vec![json!({"functionCall":{
                    "partialArgs":[{"jsonPath":"$.x", "stringValue":"nameless"}]
                }})])
            )
            .is_err());
    }

    #[test]
    fn vertex_partial_args_rejects_invalid_paths_and_bounds_buffer() {
        for path in [
            "$..x",
            "$[*]",
            "$.x[-1]",
            "$.x[65536]",
            "$.x[999999999999999999999999]",
            "$['unterminated]",
        ] {
            assert!(partial_argument_path(path).is_none(), "{path}");
        }
        assert!(partial_argument_path(&format!("${}", ".x".repeat(129))).is_none());
        let mut args = json!({});
        let mut budget = 0;
        apply_partial_argument(
            &mut args,
            &json!({"jsonPath":"$[\"a\\\"b\"][0]", "stringValue":"x"}),
            &mut budget,
        )
        .unwrap();
        assert_eq!(args, json!({"a\"b":["x"]}));
        budget = 8 * 1024 * 1024;
        assert!(apply_partial_argument(
            &mut args,
            &json!({"jsonPath":"$.x", "stringValue":"y"}),
            &mut budget
        )
        .is_err());
        let mut state = GeminiProviderState::default();
        assert!(state
            .push_line(
                &json!({}),
                partial_chunk(vec![json!({"functionCall":{
                    "partialArgs":[{"jsonPath":"$.x", "stringValue":"open", "willContinue":true}]
                }})])
            )
            .is_err());
    }

    fn terminal_frames(frames: Vec<CanonicalStreamFrame>) -> Vec<CanonicalStreamFrame> {
        frames
            .into_iter()
            .filter(|frame| {
                matches!(
                    frame.event,
                    CanonicalStreamEvent::Start
                        | CanonicalStreamEvent::Finish { .. }
                        | CanonicalStreamEvent::UnknownEvent(_)
                )
            })
            .collect()
    }

    fn observation_record(parts: Vec<Value>, finish_reason: Option<&str>) -> Value {
        let mut record = json!({
            "responseId": "resp_observation",
            "modelVersion": "gemini-2.5-pro",
            "candidates": [{"content": {"parts": parts}}],
            "usageMetadata": {
                "promptTokenCount": 22,
                "cachedContentTokenCount": 7,
                "candidatesTokenCount": 13,
                "thoughtsTokenCount": 5,
                "totalTokenCount": 40
            }
        });
        if let Some(finish_reason) = finish_reason {
            record["candidates"][0]["finishReason"] = json!(finish_reason);
        }
        record
    }

    fn assert_terminal_record_matches(
        normal: &mut GeminiProviderState,
        observer: &mut GeminiProviderState,
        record: Value,
    ) -> Vec<CanonicalStreamFrame> {
        let context = json!({"mapped_model": "fallback-model"});
        let expected = terminal_frames(
            normal
                .push_line(&context, data_line(record.clone()))
                .expect("normal provider parser"),
        );
        let actual = observer
            .push_line(&context, data_line(record))
            .expect("terminal provider parser");
        assert_eq!(
            actual, expected,
            "terminal frames must retain their full payloads"
        );
        actual
    }

    fn assert_observer_has_no_content_buffers(observer: &GeminiProviderState) {
        assert!(observer.text_parts.is_empty());
        assert!(observer.reasoning_parts.is_empty());
        assert!(observer.reasoning_signatures.is_empty());
        assert!(observer.content_parts.is_empty());
        assert!(observer.tool_calls.is_empty());
        assert!(observer.tool_results.is_empty());
    }

    #[test]
    fn gemini_terminal_observation_does_not_retain_long_stream_content() {
        let mut normal = GeminiProviderState::default();
        let mut observer = GeminiProviderState::terminal_observation();
        let media = "YQ==".repeat(4_096);
        for index in 0..32 {
            let text = format!("record-{index}:{}", "x".repeat(16_384));
            assert_terminal_record_matches(
                &mut normal,
                &mut observer,
                observation_record(
                    vec![
                        json!({"text": text}),
                        json!({"text": text, "thought": true, "thoughtSignature": media}),
                        json!({"functionCall": {"id": format!("call-{index}"), "name": "lookup", "args": {"value": text}}}),
                        json!({"functionResponse": {"name": "lookup", "response": {"result": text}}}),
                        json!({"inlineData": {"mimeType": "image/png", "data": media}}),
                        json!({"inline_data": {"mime_type": "audio/wav", "data": media}}),
                        json!({"inlineData": {"mimeType": "application/pdf", "data": media}}),
                    ],
                    None,
                ),
            );
            assert_observer_has_no_content_buffers(&observer);
            assert!(observer.observed_tool_calls);
        }
        assert!(!normal.text_parts.is_empty());
        assert!(!normal.reasoning_parts.is_empty());
        assert!(!normal.reasoning_signatures.is_empty());
        assert!(!normal.content_parts.is_empty());
        assert!(!normal.tool_calls.is_empty());
        assert!(normal
            .tool_results
            .values()
            .any(|state| state.content.len() > 16_384));
        let frames = assert_terminal_record_matches(
            &mut normal,
            &mut observer,
            observation_record(Vec::new(), Some("STOP")),
        );
        assert!(frames.iter().any(|frame| matches!(
            &frame.event,
            CanonicalStreamEvent::Finish { finish_reason: Some(reason), usage: Some(usage) }
                if reason == "tool_calls" && usage.input_tokens == 22 && usage.output_tokens == 18
                    && usage.cache_read_tokens == 7 && usage.reasoning_tokens == 5 && usage.total_tokens == 40
        )));
        assert_observer_has_no_content_buffers(&observer);
    }

    #[test]
    fn gemini_terminal_observation_matches_known_and_unknown_part_classification() {
        let parts = vec![
            Value::Null,
            json!({}),
            json!({"text": ""}),
            json!({"text": 17}),
            json!({"text": "", "thought_signature": "signature"}),
            json!({"thoughtSignature": "signature"}),
            json!({"executableCode": {"language": "python", "code": "print(1)"}}),
            json!({"codeExecutionResult": {"output": "1"}}),
            json!({"functionResponse": {}}),
            json!({"function_response": {"response": ["result"]}}),
            json!({"functionCall": {}}),
            json!({"functionCall": null}),
            json!({"inlineData": {"mimeType": "image/png", "data": "YQ=="}}),
            json!({"inline_data": {"mime_type": "audio/wav", "data": "YQ=="}}),
            json!({"inlineData": {"mimeType": "application/pdf", "data": "YQ=="}}),
            json!({"inlineData": {"mimeType": "", "data": "YQ=="}}),
            json!({"inlineData": {"mimeType": "image/png", "data": ""}}),
            json!({"file_data": {"file_uri": "gs://test/file", "mime_type": "application/pdf"}}),
            json!({"fileData": {"fileUri": ""}}),
            json!({"futurePart": {"kept": true}}),
        ];
        for part in parts {
            let mut normal = GeminiProviderState::default();
            let mut observer = GeminiProviderState::terminal_observation();
            assert_terminal_record_matches(
                &mut normal,
                &mut observer,
                json!({"responseId": "outer", "response": observation_record(vec![part], Some("STOP"))}),
            );
            assert_eq!(observer.response_id.as_deref(), Some("resp_observation"));
            assert_observer_has_no_content_buffers(&observer);
        }
    }

    #[test]
    fn gemini_terminal_observation_preserves_finish_reasons_and_eof() {
        for has_tool in [false, true] {
            for reason in [
                None,
                Some("STOP"),
                Some("MAX_TOKENS"),
                Some("RECITATION"),
                Some("FUTURE_REASON"),
            ] {
                let mut normal = GeminiProviderState::default();
                let mut observer = GeminiProviderState::terminal_observation();
                let part = if has_tool {
                    json!({"functionCall": {"name": "lookup", "args": {"query": "test"}}})
                } else {
                    json!({"text": "partial output"})
                };
                assert_terminal_record_matches(
                    &mut normal,
                    &mut observer,
                    observation_record(vec![part], None),
                );
                if let Some(reason) = reason {
                    assert_terminal_record_matches(
                        &mut normal,
                        &mut observer,
                        observation_record(Vec::new(), Some(reason)),
                    );
                }
                let context = json!({});
                assert_eq!(
                    observer.finish(&context).expect("observer EOF"),
                    terminal_frames(normal.finish(&context).expect("normal EOF"))
                );
                assert!(observer
                    .finish(&context)
                    .expect("idempotent EOF")
                    .is_empty());
                assert_observer_has_no_content_buffers(&observer);
            }
        }
    }

    #[test]
    fn gemini_terminal_observation_preserves_failure_payloads_with_usage() {
        for reason in [
            "MALFORMED_FUNCTION_CALL",
            "UNEXPECTED_TOOL_CALL",
            "TOO_MANY_TOOL_CALLS",
            "MISSING_THOUGHT_SIGNATURE",
            "MALFORMED_RESPONSE",
        ] {
            for content in [
                Value::Null,
                json!({}),
                json!({"parts": []}),
                json!({"parts": [{"text": ""}]}),
            ] {
                let mut normal = GeminiProviderState::default();
                let mut observer = GeminiProviderState::terminal_observation();
                let mut record = observation_record(Vec::new(), Some(reason));
                record["candidates"][0]["content"] = content;
                record["candidates"][0]["finishMessage"] = json!("provider failure details");
                let frames = assert_terminal_record_matches(&mut normal, &mut observer, record);
                assert!(frames.iter().any(|frame| matches!(
                    &frame.event,
                    CanonicalStreamEvent::UnknownEvent(payload)
                        if payload["response"]["error"]["code"] == reason
                            && payload["response"]["error"]["message"] == "provider failure details"
                            && payload["response"]["usage"]["input_tokens"] == 22
                )));
                assert!(observer.finish(&json!({})).expect("failed EOF").is_empty());
                assert_observer_has_no_content_buffers(&observer);
            }
        }
    }

    #[test]
    fn gemini_provider_state_emits_unknown_events_for_unknown_parts() {
        let mut state = GeminiProviderState::default();
        let report_context = json!({});
        let frames = state
            .push_line(
                &report_context,
                data_line(json!({
                    "responseId": "resp_unknown_123",
                    "modelVersion": "gemini-2.5-pro",
                    "candidates": [{
                        "index": 0,
                        "content": {
                            "parts": [
                                {
                                    "futurePart": {
                                        "kept": true
                                    }
                                }
                            ]
                        }
                    }]
                })),
            )
            .expect("unknown part should parse");

        assert!(frames.iter().any(|frame| matches!(
            frame.event,
            CanonicalStreamEvent::UnknownEvent(ref payload)
                if payload.get("futurePart").is_some()
        )));
    }

    #[test]
    fn gemini_provider_state_parses_thoughts_code_and_content_filter_finish() {
        let mut state = GeminiProviderState::default();
        let report_context = json!({});
        let frames = state
            .push_line(
                &report_context,
                data_line(json!({
                    "responseId": "resp_123",
                    "modelVersion": "gemini-2.5-pro",
                    "candidates": [{
                        "index": 0,
                        "finishReason": "RECITATION",
                        "content": {
                            "parts": [
                                { "text": "reason", "thought": true, "thoughtSignature": "sig_123" },
                                { "executableCode": { "language": "python", "code": "print(1)" } }
                            ]
                        }
                    }],
                    "usageMetadata": {
                        "promptTokenCount": 5,
                        "cachedContentTokenCount": 4,
                        "candidatesTokenCount": 2,
                        "thoughtsTokenCount": 4,
                        "totalTokenCount": 11
                    }
                })),
            )
            .expect("chunk should parse");

        assert!(matches!(frames[0].event, CanonicalStreamEvent::Start));
        assert!(frames.iter().any(|frame| matches!(
            frame.event,
            CanonicalStreamEvent::ReasoningDelta(ref text) if text == "reason"
        )));
        assert!(frames.iter().any(|frame| matches!(
            frame.event,
            CanonicalStreamEvent::ReasoningSignature(ref signature) if signature == "sig_123"
        )));
        assert!(frames.iter().any(|frame| matches!(
            frame.event,
            CanonicalStreamEvent::TextDelta(ref text) if text == "```python\nprint(1)\n```"
        )));
        assert!(frames.iter().any(|frame| matches!(
            frame.event,
            CanonicalStreamEvent::Finish { ref finish_reason, .. }
                if finish_reason.as_deref() == Some("content_filter")
        )));
        assert!(frames.iter().any(|frame| matches!(
            frame.event,
            CanonicalStreamEvent::Finish {
                usage: Some(CanonicalUsage {
                    input_tokens: 5,
                    input_tokens_include_cache: true,
                    output_tokens: 6,
                    cache_read_tokens: 4,
                    reasoning_tokens: 4,
                    total_tokens: 11,
                    ..
                }),
                ..
            }
        )));
    }

    #[test]
    fn gemini_provider_state_preserves_signature_only_reasoning_terminal() {
        let mut state = GeminiProviderState::default();
        let report_context = json!({});
        let frames = state
            .push_line(
                &report_context,
                data_line(json!({
                    "response": {
                        "responseId": "resp_signature_only_123",
                        "modelVersion": "gemini-3.7-flash-tiered",
                        "candidates": [{
                            "index": 0,
                            "finishReason": "MAX_TOKENS",
                            "content": {
                                "role": "model",
                                "parts": [{
                                    "text": "",
                                    "thoughtSignature": "opaque-thought-signature"
                                }]
                            }
                        }],
                        "usageMetadata": {
                            "promptTokenCount": 22,
                            "thoughtsTokenCount": 29,
                            "totalTokenCount": 51
                        }
                    },
                    "traceId": "trace-signature-only"
                })),
            )
            .expect("signature-only reasoning terminal should parse");

        assert!(frames.iter().any(|frame| matches!(
            frame.event,
            CanonicalStreamEvent::ReasoningSignature(ref signature)
                if signature == "opaque-thought-signature"
        )));
        assert!(frames.iter().any(|frame| matches!(
            frame.event,
            CanonicalStreamEvent::Finish {
                ref finish_reason,
                usage: Some(CanonicalUsage {
                    reasoning_tokens: 29,
                    ..
                }),
            } if finish_reason.as_deref() == Some("length")
        )));
    }

    #[test]
    fn gemini_provider_state_emits_terminal_error_for_malformed_function_call() {
        let mut state = GeminiProviderState::default();
        let report_context = json!({});
        let frames = state
            .push_line(
                &report_context,
                data_line(json!({
                    "response": {
                        "responseId": "resp_malformed_tool_call",
                        "modelVersion": "gemini-3.7-flash-tiered",
                        "candidates": [{
                            "index": 0,
                            "content": {
                                "role": "model",
                                "parts": [{
                                    "text": "",
                                    "thoughtSignature": "opaque-thought-signature"
                                }]
                            },
                            "finishReason": "MALFORMED_FUNCTION_CALL",
                            "finishMessage": "Malformed function call: Function call is empty - no input to parse."
                        }],
                        "usageMetadata": {
                            "promptTokenCount": 206744,
                            "cachedContentTokenCount": 203947,
                            "thoughtsTokenCount": 1130,
                            "totalTokenCount": 207874
                        }
                    }
                })),
            )
            .expect("malformed function call terminal should parse");

        assert!(frames.iter().any(|frame| matches!(
            &frame.event,
            CanonicalStreamEvent::UnknownEvent(payload)
                if payload["type"] == "response.failed"
                    && payload["response"]["status"] == "failed"
                    && payload["response"]["id"] == "resp_malformed_tool_call"
                    && payload["response"]["model"] == "gemini-3.7-flash-tiered"
                    && payload["response"]["error"]["code"] == "MALFORMED_FUNCTION_CALL"
                    && payload["response"]["error"]["message"]
                        == "Malformed function call: Function call is empty - no input to parse."
                    && payload["response"]["usage"]["input_tokens"] == 206744
                    && payload["response"]["usage"]["output_tokens"] == 1130
                    && payload["response"]["usage"]["total_tokens"] == 207874
        )));
        assert!(!frames
            .iter()
            .any(|frame| matches!(frame.event, CanonicalStreamEvent::Finish { .. })));
        assert!(state
            .finish(&report_context)
            .expect("finished error stream should not synthesize success")
            .is_empty());
    }

    #[test]
    fn gemini_provider_state_parses_function_response_as_tool_result() {
        let mut state = GeminiProviderState::default();
        let report_context = json!({});
        let frames = state
            .push_line(
                &report_context,
                data_line(json!({
                    "responseId": "resp_tool_result_123",
                    "modelVersion": "gemini-2.5-pro",
                    "candidates": [{
                        "index": 0,
                        "content": {
                            "parts": [{
                                "functionResponse": {
                                    "id": "call_123",
                                    "name": "lookup",
                                    "response": {"ok": true}
                                }
                            }]
                        }
                    }]
                })),
            )
            .expect("function response should parse");

        assert!(frames.iter().any(|frame| matches!(
            frame.event,
            CanonicalStreamEvent::ToolResultDelta {
                index: 0,
                ref tool_use_id,
                name: Some(ref name),
                ref content,
            } if tool_use_id == "call_123" && name == "lookup" && content == "{\"ok\":true}"
        )));
    }

    #[test]
    fn gemini_provider_state_preserves_function_call_thought_signature() {
        let mut state = GeminiProviderState::default();
        let report_context = json!({});
        let frames = state
            .push_line(
                &report_context,
                data_line(json!({
                    "responseId": "resp_signed_tool_123",
                    "modelVersion": "gemini-3-flash-preview",
                    "candidates": [{
                        "index": 0,
                        "content": {
                            "parts": [{
                                "functionCall": {
                                    "id": "call_123",
                                    "name": "lookup",
                                    "args": {"query": "rust"}
                                },
                                "thoughtSignature": "opaque-tool-signature"
                            }]
                        }
                    }]
                })),
            )
            .expect("signed function call should parse");

        let signature_index = frames
            .iter()
            .position(|frame| {
                matches!(
                    frame.event,
                    CanonicalStreamEvent::ToolCallSignature {
                        index: 0,
                        ref signature,
                    } if signature == "opaque-tool-signature"
                )
            })
            .expect("tool signature event");
        let call_index = frames
            .iter()
            .position(|frame| {
                matches!(
                    frame.event,
                    CanonicalStreamEvent::ToolCallStart { index: 0, .. }
                )
            })
            .expect("tool call start event");
        assert!(signature_index < call_index);
    }

    #[test]
    fn gemini_provider_state_keeps_parallel_function_calls_from_separate_chunks() {
        let mut state = GeminiProviderState::default();
        let report_context = json!({});
        let chunk = |call: Value| {
            data_line(json!({
                "responseId": "resp_parallel_123",
                "modelVersion": "gemini-3.8-flash",
                "candidates": [{
                    "index": 0,
                    "content": {"role": "model", "parts": [{"functionCall": call}]}
                }]
            }))
        };
        let mut frames = Vec::new();
        for call in [
            json!({"id": "call_a", "name": "get_weather", "args": {"city": "Paris"}}),
            json!({"id": "call_b", "name": "get_weather", "args": {"city": "Tokyo"}}),
            json!({"name": "get_time", "args": {"city": "Paris"}}),
            json!({"name": "get_time", "args": {"city": "Tokyo"}}),
        ] {
            frames.extend(
                state
                    .push_line(&report_context, chunk(call))
                    .expect("function call chunk should parse"),
            );
        }

        let starts = frames
            .iter()
            .filter_map(|frame| match &frame.event {
                CanonicalStreamEvent::ToolCallStart {
                    index,
                    call_id,
                    name,
                } => Some((*index, call_id.clone(), name.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(starts.len(), 4);
        assert_eq!(
            starts
                .iter()
                .map(|(index, _, _)| *index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(starts[0].1, "call_a");
        assert_eq!(starts[1].1, "call_b");
        assert_eq!(
            starts
                .iter()
                .map(|(_, _, name)| name.as_str())
                .collect::<Vec<_>>(),
            vec!["get_weather", "get_weather", "get_time", "get_time"]
        );
        assert_ne!(starts[2].1, starts[3].1);

        let mut arguments = BTreeMap::<usize, String>::new();
        for frame in &frames {
            if let CanonicalStreamEvent::ToolCallArgumentsDelta {
                index,
                arguments: delta,
            } = &frame.event
            {
                arguments.entry(*index).or_default().push_str(delta);
            }
        }
        assert_eq!(arguments[&0], "{\"city\":\"Paris\"}");
        assert_eq!(arguments[&1], "{\"city\":\"Tokyo\"}");
        assert_eq!(arguments[&2], "{\"city\":\"Paris\"}");
        assert_eq!(arguments[&3], "{\"city\":\"Tokyo\"}");
    }

    #[test]
    fn gemini_client_emitter_marks_reasoning_parts_as_thoughts() {
        let mut emitter = GeminiClientEmitter::default();
        let mut bytes = emitter
            .emit(CanonicalStreamFrame {
                id: "resp_123".to_string(),
                model: "gemini-2.5-pro".to_string(),
                event: CanonicalStreamEvent::ReasoningDelta("reason".to_string()),
            })
            .expect("reasoning should encode");
        bytes.extend(
            emitter
                .emit(CanonicalStreamFrame {
                    id: "resp_123".to_string(),
                    model: "gemini-2.5-pro".to_string(),
                    event: CanonicalStreamEvent::ReasoningSignature("sig_123".to_string()),
                })
                .expect("signature should encode"),
        );
        bytes.extend(
            emitter
                .emit(CanonicalStreamFrame {
                    id: "resp_123".to_string(),
                    model: "gemini-2.5-pro".to_string(),
                    event: CanonicalStreamEvent::Finish {
                        finish_reason: Some("stop".to_string()),
                        usage: Some(CanonicalUsage {
                            input_tokens: 1,
                            output_tokens: 3,
                            reasoning_tokens: 1,
                            total_tokens: 4,
                            cache_read_tokens: 5,
                            ..CanonicalUsage::default()
                        }),
                    },
                })
                .expect("finish should encode"),
        );

        let sse = String::from_utf8(bytes).expect("sse should be utf8");
        assert!(sse.contains("\"thought\":true"));
        assert!(sse.contains("\"thoughtSignature\":\"sig_123\""));
        assert!(sse.contains("\"promptTokenCount\":6"));
        assert!(sse.contains("\"thoughtsTokenCount\":1"));
        assert!(sse.contains("\"candidatesTokenCount\":2"));
        assert!(sse.contains("\"cachedContentTokenCount\":5"));
        assert!(sse.contains("\"totalTokenCount\":9"));
        assert!(sse.contains("\"finishReason\":\"STOP\""));
    }

    #[test]
    fn gemini_provider_state_parses_inline_image_parts() {
        let mut state = GeminiProviderState::default();
        let report_context = json!({});
        let frames = state
            .push_line(
                &report_context,
                data_line(json!({
                    "responseId": "resp_media_123",
                    "modelVersion": "gemini-2.5-pro",
                    "candidates": [{
                        "index": 0,
                        "content": {
                            "parts": [
                                { "inlineData": { "mimeType": "image/png", "data": "iVBORw0KGgo=" } }
                            ]
                        }
                    }]
                })),
            )
            .expect("chunk should parse");

        assert!(frames.iter().any(|frame| matches!(
            frame.event,
            CanonicalStreamEvent::ContentPart(CanonicalContentPart::ImageUrl(ref url))
                if url == "data:image/png;base64,iVBORw0KGgo="
        )));
    }

    #[test]
    fn gemini_client_emitter_emits_inline_image_parts() {
        let mut emitter = GeminiClientEmitter::default();
        let bytes = emitter
            .emit(CanonicalStreamFrame {
                id: "resp_img_123".to_string(),
                model: "gemini-2.5-pro".to_string(),
                event: CanonicalStreamEvent::ContentPart(CanonicalContentPart::ImageUrl(
                    "data:image/png;base64,iVBORw0KGgo=".to_string(),
                )),
            })
            .expect("image should encode");

        let sse = String::from_utf8(bytes).expect("sse should be utf8");
        assert!(
            sse.contains("\"inlineData\":{\"mimeType\":\"image/png\",\"data\":\"iVBORw0KGgo=\"}")
        );
    }

    #[test]
    fn gemini_client_emitter_emits_function_response_for_tool_results() {
        let mut emitter = GeminiClientEmitter::default();
        let bytes = emitter
            .emit(CanonicalStreamFrame {
                id: "resp_tool_result_123".to_string(),
                model: "gemini-2.5-pro".to_string(),
                event: CanonicalStreamEvent::ToolResultDelta {
                    index: 2,
                    tool_use_id: "call_123".to_string(),
                    name: Some("lookup".to_string()),
                    content: "{\"ok\":true}".to_string(),
                },
            })
            .expect("tool result should encode");

        let sse = String::from_utf8(bytes).expect("sse should be utf8");
        assert!(sse.contains("\"functionResponse\""));
        assert!(sse.contains("\"id\":\"call_123\""));
        assert!(sse.contains("\"name\":\"lookup\""));
        assert!(sse.contains("\"response\":{\"ok\":true}"));
    }
}
