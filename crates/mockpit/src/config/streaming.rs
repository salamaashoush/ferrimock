//! Declarative `sse:` / `ws:` mock configuration and its lowering into
//! the engine's [`SseScript`]/[`WsScript`] types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::response::parse_duration;
use crate::types::{
    SseData, SseEvent, SseRepeat, SseScript, WsAction, WsMessageMatch, WsPayload, WsRule, WsScript,
    template_hash,
};

/// `sse:` block — ordered event playback with per-event delays.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SseConfig {
    /// Initial `retry:` field (milliseconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<u32>,

    /// Comment-ping interval (duration string, e.g. "15s").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<String>,

    /// How many times the event list plays: integer, `"forever"`, or `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<SseRepeatConfig>,

    /// Close the connection after playback (default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_after: Option<bool>,

    /// Real SSE endpoint (http:// or https://) to relay instead of
    /// playing back `events` — exclusive with every playback field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<SseEventConfig>,
}

/// Playback repetition: `3`, `"forever"`, or `true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SseRepeatConfig {
    Count(u32),
    Word(String),
    Forever(bool),
}

/// One SSE event: a bare string (data-only) or a full object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SseEventConfig {
    Bare(String),
    Full(SseEventFull),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SseEventFull {
    /// Event name (`event:` field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,

    /// Event id (`id:` field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Per-event `retry:` field (milliseconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<u32>,

    /// Sleep before emitting this event (duration string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<String>,

    /// Static data; objects/arrays are serialized to compact JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,

    /// Tera template rendered per emission (mutually exclusive with `data`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_template: Option<String>,
}

impl SseConfig {
    pub fn into_script(self) -> crate::Result<SseScript> {
        if let Some(upstream) = self.upstream {
            if !upstream.starts_with("http://") && !upstream.starts_with("https://") {
                return Err(crate::mp_err!(
                    "sse.upstream must be an http:// or https:// URL, got '{upstream}'"
                ));
            }
            if !self.events.is_empty()
                || self.retry.is_some()
                || self.keep_alive.is_some()
                || self.repeat.is_some()
                || self.close_after.is_some()
            {
                return Err(crate::mp_err!(
                    "sse.upstream relays the real stream and cannot combine with \
                     events/retry/keep_alive/repeat/close_after"
                ));
            }
            return Ok(SseScript {
                retry: None,
                keep_alive: None,
                repeat: SseRepeat::Count(1),
                close_after: true,
                upstream: Some(upstream),
                events: Vec::new(),
            });
        }

        if self.events.is_empty() {
            return Err(crate::mp_err!(
                "sse: requires at least one event (or an `upstream` URL)"
            ));
        }

        let keep_alive = self
            .keep_alive
            .as_deref()
            .map(parse_duration)
            .transpose()
            .map_err(|e| crate::mp_err!("Invalid sse.keep_alive: {e}"))?;

        let repeat = match self.repeat {
            None | Some(SseRepeatConfig::Forever(false)) => SseRepeat::Count(1),
            Some(SseRepeatConfig::Count(n)) => SseRepeat::Count(n.max(1)),
            Some(SseRepeatConfig::Forever(true)) => SseRepeat::Forever,
            Some(SseRepeatConfig::Word(word)) if word.eq_ignore_ascii_case("forever") => {
                SseRepeat::Forever
            }
            Some(SseRepeatConfig::Word(word)) => {
                return Err(crate::mp_err!(
                    "Invalid sse.repeat '{word}': expected an integer or \"forever\""
                ));
            }
        };

        let events = self
            .events
            .into_iter()
            .map(SseEventConfig::into_event)
            .collect::<crate::Result<Vec<_>>>()?;

        Ok(SseScript {
            retry: self.retry,
            keep_alive,
            repeat,
            close_after: self.close_after.unwrap_or(true),
            upstream: None,
            events,
        })
    }
}

impl SseEventConfig {
    fn into_event(self) -> crate::Result<SseEvent> {
        match self {
            Self::Bare(data) => Ok(SseEvent {
                event: None,
                id: None,
                retry: None,
                delay: None,
                data: SseData::Static(data),
            }),
            Self::Full(full) => {
                let delay = full
                    .delay
                    .as_deref()
                    .map(parse_duration)
                    .transpose()
                    .map_err(|e| crate::mp_err!("Invalid sse event delay: {e}"))?;
                let data = match (full.data, full.data_template) {
                    (Some(_), Some(_)) => {
                        return Err(crate::mp_err!(
                            "sse event cannot set both `data` and `data_template`"
                        ));
                    }
                    (Some(value), None) => SseData::Static(value_to_data(&value)),
                    (None, Some(source)) => {
                        let hash = template_hash(&source);
                        SseData::Template { source, hash }
                    }
                    (None, None) => {
                        if full.retry.is_none() {
                            return Err(crate::mp_err!(
                                "sse event needs `data`, `data_template`, or `retry`"
                            ));
                        }
                        SseData::Static(String::new())
                    }
                };
                Ok(SseEvent {
                    event: full.event,
                    id: full.id,
                    retry: full.retry,
                    delay,
                    data,
                })
            }
        }
    }
}

fn value_to_data(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `ws:` block — connect-time actions plus first-match-wins message rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct WsConfig {
    /// Subprotocol negotiated on the 101 response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subprotocol: Option<String>,

    /// Echo unmatched messages back to the client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<bool>,

    /// Real upstream WebSocket URL for passthrough (`forward` actions
    /// and unmatched-message forwarding).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_connect: Vec<WsActionConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_message: Vec<WsRuleConfig>,
}

/// A single WS action: `echo` / `forward` words, or one-key objects
/// (`send`, `send_template`, `send_binary`, `delay`, `close`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WsActionConfig {
    Word(WsActionWord),
    Send { send: Value },
    SendTemplate { send_template: String },
    SendBinary { send_binary: String },
    Delay { delay: String },
    Close { close: WsCloseConfig },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum WsActionWord {
    Echo,
    Forward,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct WsCloseConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct WsRuleConfig {
    #[serde(rename = "match")]
    pub match_config: WsMatchConfig,
    pub actions: Vec<WsActionConfig>,
}

/// Message selector: exactly one of `exact` / `regex` / `json_path` /
/// `binary_base64` / `binary_prefix_base64` / `any` must be set.
///
/// `equals` refines `json_path`. `exact`/`regex`/`json_path` apply to
/// text frames only; the `binary_*` selectors are the byte-frame
/// counterparts and `any` matches both.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct WsMatchConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<Value>,
    /// Whole binary frame equals these base64-decoded bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_base64: Option<String>,
    /// Binary frame starts with these base64-decoded bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_prefix_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub any: Option<bool>,
}

impl WsConfig {
    pub fn into_script(self) -> crate::Result<WsScript> {
        let has_upstream = self.upstream.is_some();
        if let Some(upstream) = &self.upstream
            && !upstream.starts_with("ws://")
            && !upstream.starts_with("wss://")
        {
            return Err(crate::mp_err!(
                "ws.upstream must be a ws:// or wss:// URL, got '{upstream}'"
            ));
        }

        let on_connect = self
            .on_connect
            .into_iter()
            .map(|a| a.into_action(has_upstream))
            .collect::<crate::Result<Vec<_>>>()?;
        let on_message = self
            .on_message
            .into_iter()
            .map(|r| r.into_rule(has_upstream))
            .collect::<crate::Result<Vec<_>>>()?;

        Ok(WsScript {
            subprotocol: self.subprotocol,
            echo: self.echo.unwrap_or(false),
            upstream: self.upstream,
            on_connect,
            on_message,
        })
    }
}

impl WsActionConfig {
    fn into_action(self, has_upstream: bool) -> crate::Result<WsAction> {
        match self {
            Self::Word(WsActionWord::Echo) => Ok(WsAction::Echo),
            Self::Word(WsActionWord::Forward) => {
                if !has_upstream {
                    return Err(crate::mp_err!(
                        "ws `forward` action requires `upstream` to be set"
                    ));
                }
                Ok(WsAction::Forward)
            }
            Self::Send { send } => Ok(WsAction::Send(WsPayload::Text(value_to_data(&send)))),
            Self::SendTemplate { send_template } => {
                let hash = template_hash(&send_template);
                Ok(WsAction::Send(WsPayload::Template {
                    source: send_template,
                    hash,
                }))
            }
            Self::SendBinary { send_binary } => {
                use base64::Engine as _;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(send_binary.trim())
                    .map_err(|e| crate::mp_err!("Invalid ws send_binary base64: {e}"))?;
                Ok(WsAction::Send(WsPayload::Binary(bytes.into())))
            }
            Self::Delay { delay } => {
                let duration =
                    parse_duration(&delay).map_err(|e| crate::mp_err!("Invalid ws delay: {e}"))?;
                Ok(WsAction::Delay(duration))
            }
            Self::Close { close } => {
                if let Some(code) = close.code
                    && !(1000..=4999).contains(&code)
                {
                    return Err(crate::mp_err!(
                        "Invalid ws close code {code}: expected 1000..=4999"
                    ));
                }
                Ok(WsAction::Close {
                    code: close.code,
                    reason: close.reason,
                })
            }
        }
    }
}

impl WsRuleConfig {
    fn into_rule(self, has_upstream: bool) -> crate::Result<WsRule> {
        let matcher = self.match_config.into_matcher()?;
        let actions = self
            .actions
            .into_iter()
            .map(|a| a.into_action(has_upstream))
            .collect::<crate::Result<Vec<_>>>()?;
        if actions.is_empty() {
            return Err(crate::mp_err!(
                "ws on_message rule needs at least one action"
            ));
        }
        Ok(WsRule { matcher, actions })
    }
}

impl WsMatchConfig {
    fn into_matcher(self) -> crate::Result<WsMessageMatch> {
        let selectors = usize::from(self.exact.is_some())
            + usize::from(self.regex.is_some())
            + usize::from(self.json_path.is_some())
            + usize::from(self.binary_base64.is_some())
            + usize::from(self.binary_prefix_base64.is_some())
            + usize::from(self.any.is_some());
        if selectors != 1 {
            return Err(crate::mp_err!(
                "ws message match needs exactly one of `exact`, `regex`, `json_path`, \
                 `binary_base64`, `binary_prefix_base64`, `any`"
            ));
        }
        if self.equals.is_some() && self.json_path.is_none() {
            return Err(crate::mp_err!(
                "ws message match `equals` requires `json_path`"
            ));
        }

        if let Some(exact) = self.exact {
            return Ok(WsMessageMatch::Exact(exact));
        }
        if let Some(pattern) = self.regex {
            let re = regex::Regex::new(&pattern)
                .map_err(|e| crate::mp_err!("Invalid ws message regex '{pattern}': {e}"))?;
            return Ok(WsMessageMatch::Regex(re));
        }
        if let Some(path) = self.json_path {
            return Ok(WsMessageMatch::JsonPath {
                path,
                equals: self.equals,
            });
        }
        if let Some(encoded) = self.binary_base64 {
            return Ok(WsMessageMatch::Binary {
                bytes: decode_binary_match(&encoded, "binary_base64")?,
                prefix: false,
            });
        }
        if let Some(encoded) = self.binary_prefix_base64 {
            return Ok(WsMessageMatch::Binary {
                bytes: decode_binary_match(&encoded, "binary_prefix_base64")?,
                prefix: true,
            });
        }
        Ok(WsMessageMatch::Any)
    }
}

fn decode_binary_match(encoded: &str, field: &str) -> crate::Result<bytes::Bytes> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map(bytes::Bytes::from)
        .map_err(|e| crate::mp_err!("Invalid ws {field} base64: {e}"))
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for SseRepeatConfig {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SseRepeatConfig".into()
    }

    fn json_schema(_schema_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let value = serde_json::json!({
            "description": "Playback repetition: integer count, \"forever\", or true",
            "oneOf": [
                { "type": "integer", "minimum": 1 },
                { "type": "string", "enum": ["forever"] },
                { "type": "boolean" }
            ]
        });
        object_schema(value)
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for SseEventConfig {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SseEventConfig".into()
    }

    fn json_schema(schema_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let full = schema_gen.subschema_for::<SseEventFull>();
        let value = serde_json::json!({
            "description": "SSE event: bare string (data-only) or full object",
            "oneOf": [
                { "type": "string", "description": "Data-only event" },
                full
            ]
        });
        object_schema(value)
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for WsActionConfig {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "WsActionConfig".into()
    }

    fn json_schema(_schema_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let value = serde_json::json!({
            "description": "WebSocket action",
            "oneOf": [
                { "type": "string", "enum": ["echo", "forward"] },
                { "type": "object", "properties": { "send": { "description": "Payload; objects are JSON-stringified" } }, "required": ["send"], "additionalProperties": false },
                { "type": "object", "properties": { "send_template": { "type": "string", "description": "Tera template; message available as {{ body }} / {{ body_json }}" } }, "required": ["send_template"], "additionalProperties": false },
                { "type": "object", "properties": { "send_binary": { "type": "string", "description": "Base64-encoded binary frame" } }, "required": ["send_binary"], "additionalProperties": false },
                { "type": "object", "properties": { "delay": { "type": "string", "description": "Duration, e.g. 100ms" } }, "required": ["delay"], "additionalProperties": false },
                { "type": "object", "properties": { "close": { "type": "object", "properties": { "code": { "type": "integer", "minimum": 1000, "maximum": 4999 }, "reason": { "type": "string" } } } }, "required": ["close"], "additionalProperties": false }
            ]
        });
        object_schema(value)
    }
}

#[cfg(feature = "schema")]
fn object_schema(value: serde_json::Value) -> schemars::Schema {
    if let serde_json::Value::Object(map) = value {
        map.into()
    } else {
        serde_json::Map::new().into()
    }
}
