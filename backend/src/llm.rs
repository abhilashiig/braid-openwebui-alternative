//! Provider-neutral chat format, translation to/from OpenAI and Anthropic, and the upstream call with
//! retries and key failover. Streams are translated chunk by chunk, never buffered whole.

use std::{pin::Pin, time::Duration};

use eventsource_stream::Eventsource;
use futures::{Stream, StreamExt, stream};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    state::AppState,
    upstream::{self, Provider},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text { text: String },
    /// `url` is an https URL or a `data:<mime>;base64,...` URL.
    Image { url: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON-encoded arguments.
    pub arguments: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Msg {
    pub role: Role,
    pub content: Vec<Part>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Msg {
    pub fn joined_text(&self) -> String {
        self.content
            .iter()
            .filter_map(|p| if let Part::Text { text } = p { Some(text.as_str()) } else { None })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, Default)]
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Msg>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<i64>,
    pub stop: Vec<String>,
    pub tools: Vec<ToolDef>,
    pub stream: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    /// Total input tokens including cached ones (OpenAI semantics).
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
}

impl Usage {
    fn merge(&mut self, o: Usage) {
        self.input_tokens = o.input_tokens.or(self.input_tokens);
        self.output_tokens = o.output_tokens.or(self.output_tokens);
        self.cached_tokens = o.cached_tokens.or(self.cached_tokens);
        self.cache_write_tokens = o.cache_write_tokens.or(self.cache_write_tokens);
        self.reasoning_tokens = o.reasoning_tokens.or(self.reasoning_tokens);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Text(String),
    Reasoning(String),
    ToolCallStart { index: usize, id: String, name: String },
    ToolCallArgs { index: usize, delta: String },
    Usage(Usage),
    /// Normalized: stop | length | tool_calls | content_filter
    Finish(String),
    /// Provider-reported speeds (llama.cpp style `timings`).
    Timings { prompt_tps: Option<f64>, output_tps: Option<f64> },
}

#[derive(Clone, Debug, Serialize)]
pub struct UpstreamError {
    /// HTTP status to report to our own callers.
    pub status: u16,
    /// rate_limit | context_length | auth | unavailable | timeout | bad_request | error
    pub kind: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl UpstreamError {
    fn new(status: u16, kind: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self { status, kind, message: message.into(), retryable }
    }
}

pub type EventStream = Pin<Box<dyn Stream<Item = Result<Event, UpstreamError>> + Send>>;

// ---------- request building ----------

fn split_data_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime, data))
}

fn openai_body(req: &ChatRequest, p: &Provider) -> Value {
    let mut messages = vec![];
    if let Some(s) = req.system.as_deref().filter(|s| !s.is_empty()) {
        messages.push(json!({ "role": "system", "content": s }));
    }
    for m in &req.messages {
        match m.role {
            Role::Tool => messages.push(json!({
                "role": "tool", "tool_call_id": m.tool_call_id, "content": m.joined_text()
            })),
            Role::Assistant => {
                let mut v = json!({ "role": "assistant", "content": m.joined_text() });
                if !m.tool_calls.is_empty() {
                    v["tool_calls"] = json!(m.tool_calls.iter().map(|t| json!({
                        "id": t.id, "type": "function", "function": { "name": t.name, "arguments": t.arguments }
                    })).collect::<Vec<_>>());
                }
                messages.push(v);
            }
            Role::User => {
                let simple = m.content.iter().all(|p| matches!(p, Part::Text { .. }));
                let content = if simple {
                    json!(m.joined_text())
                } else {
                    json!(m.content.iter().map(|p| match p {
                        Part::Text { text } => json!({ "type": "text", "text": text }),
                        Part::Image { url } => json!({ "type": "image_url", "image_url": { "url": url } }),
                    }).collect::<Vec<_>>())
                };
                messages.push(json!({ "role": "user", "content": content }));
            }
        }
    }
    let mut body = json!({ "model": req.model, "messages": messages, "stream": req.stream });
    if req.stream {
        body["stream_options"] = json!({ "include_usage": true });
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(t) = req.top_p {
        body["top_p"] = json!(t);
    }
    if let Some(n) = req.max_tokens {
        // OpenAI's own API rejects max_tokens for reasoning models; compatible servers mostly only know max_tokens.
        let key = if p.base_url.contains("api.openai.com") { "max_completion_tokens" } else { "max_tokens" };
        body[key] = json!(n);
    }
    if !req.stop.is_empty() {
        body["stop"] = json!(req.stop);
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(req.tools.iter().map(|t| json!({
            "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
        })).collect::<Vec<_>>());
    }
    body
}

fn anthropic_body(req: &ChatRequest) -> Value {
    let mut messages: Vec<Value> = vec![];
    for m in &req.messages {
        let (role, blocks): (&str, Vec<Value>) = match m.role {
            Role::Tool => (
                "user",
                vec![json!({ "type": "tool_result", "tool_use_id": m.tool_call_id, "content": m.joined_text() })],
            ),
            Role::Assistant => {
                let mut b = vec![];
                let text = m.joined_text();
                if !text.is_empty() {
                    b.push(json!({ "type": "text", "text": text }));
                }
                for t in &m.tool_calls {
                    let input: Value = serde_json::from_str(&t.arguments).unwrap_or_else(|_| json!({}));
                    b.push(json!({ "type": "tool_use", "id": t.id, "name": t.name, "input": input }));
                }
                ("assistant", b)
            }
            Role::User => (
                "user",
                m.content
                    .iter()
                    .map(|p| match p {
                        Part::Text { text } => json!({ "type": "text", "text": text }),
                        Part::Image { url } => match split_data_url(url) {
                            Some((mime, data)) => {
                                json!({ "type": "image", "source": { "type": "base64", "media_type": mime, "data": data } })
                            }
                            None => json!({ "type": "image", "source": { "type": "url", "url": url } }),
                        },
                    })
                    .filter(|b| b["type"] != "text" || b["text"].as_str().is_some_and(|t| !t.is_empty()))
                    .collect(),
            ),
        };
        if blocks.is_empty() {
            continue;
        }
        // Anthropic requires alternating roles; merge consecutive same-role turns.
        match messages.last_mut() {
            Some(last) if last["role"] == role => last["content"].as_array_mut().unwrap().extend(blocks),
            _ => messages.push(json!({ "role": role, "content": blocks })),
        }
    }
    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "max_tokens": req.max_tokens.unwrap_or(8192),
        "stream": req.stream,
    });
    if let Some(s) = req.system.as_deref().filter(|s| !s.is_empty()) {
        body["system"] = json!(s);
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t.min(1.0));
    }
    if let Some(t) = req.top_p {
        body["top_p"] = json!(t);
    }
    if !req.stop.is_empty() {
        body["stop_sequences"] = json!(req.stop);
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(req.tools.iter().map(|t| json!({
            "name": t.name, "description": t.description, "input_schema": t.parameters
        })).collect::<Vec<_>>());
    }
    body
}

// ---------- response parsing ----------

fn int(v: &Value) -> Option<i64> {
    v.as_i64()
}

pub fn openai_usage(u: &Value) -> Usage {
    Usage {
        input_tokens: int(&u["prompt_tokens"]),
        output_tokens: int(&u["completion_tokens"]),
        cached_tokens: int(&u["prompt_tokens_details"]["cached_tokens"]),
        cache_write_tokens: None,
        reasoning_tokens: int(&u["completion_tokens_details"]["reasoning_tokens"]),
    }
}

pub fn anthropic_usage(u: &Value) -> Usage {
    let read = int(&u["cache_read_input_tokens"]);
    let write = int(&u["cache_creation_input_tokens"]);
    Usage {
        input_tokens: int(&u["input_tokens"]).map(|i| i + read.unwrap_or(0) + write.unwrap_or(0)),
        output_tokens: int(&u["output_tokens"]),
        cached_tokens: read,
        cache_write_tokens: write,
        reasoning_tokens: None,
    }
}

fn openai_finish(r: &str) -> String {
    match r {
        "tool_calls" | "function_call" => "tool_calls",
        "length" => "length",
        "content_filter" => "content_filter",
        _ => "stop",
    }
    .into()
}

fn anthropic_finish(r: &str) -> String {
    match r {
        "tool_use" => "tool_calls",
        "max_tokens" => "length",
        "refusal" => "content_filter",
        _ => "stop",
    }
    .into()
}

fn openai_chunk(v: &Value) -> Vec<Event> {
    let mut out = vec![];
    if let Some(choice) = v["choices"].get(0) {
        let d = &choice["delta"];
        for key in ["reasoning_content", "reasoning"] {
            if let Some(r) = d[key].as_str().filter(|s| !s.is_empty()) {
                out.push(Event::Reasoning(r.into()));
                break;
            }
        }
        if let Some(t) = d["content"].as_str().filter(|s| !s.is_empty()) {
            out.push(Event::Text(t.into()));
        }
        for (i, tc) in d["tool_calls"].as_array().into_iter().flatten().enumerate() {
            let index = tc["index"].as_u64().map_or(i, |x| x as usize);
            if let Some(name) = tc["function"]["name"].as_str() {
                out.push(Event::ToolCallStart {
                    index,
                    id: tc["id"].as_str().map_or_else(|| format!("call_{index}"), str::to_string),
                    name: name.into(),
                });
            }
            if let Some(a) = tc["function"]["arguments"].as_str().filter(|s| !s.is_empty()) {
                out.push(Event::ToolCallArgs { index, delta: a.into() });
            }
        }
        if let Some(r) = choice["finish_reason"].as_str() {
            out.push(Event::Finish(openai_finish(r)));
        }
    }
    if v["usage"].is_object() {
        out.push(Event::Usage(openai_usage(&v["usage"])));
    }
    if let Some(t) = v["timings"].as_object() {
        out.push(Event::Timings {
            prompt_tps: t.get("prompt_per_second").and_then(Value::as_f64),
            output_tps: t.get("predicted_per_second").and_then(Value::as_f64),
        });
    }
    out
}

fn openai_full(v: &Value) -> Vec<Event> {
    let mut out = vec![];
    let m = &v["choices"][0]["message"];
    for key in ["reasoning_content", "reasoning"] {
        if let Some(r) = m[key].as_str().filter(|s| !s.is_empty()) {
            out.push(Event::Reasoning(r.into()));
            break;
        }
    }
    if let Some(t) = m["content"].as_str().filter(|s| !s.is_empty()) {
        out.push(Event::Text(t.into()));
    }
    for (index, tc) in m["tool_calls"].as_array().into_iter().flatten().enumerate() {
        out.push(Event::ToolCallStart {
            index,
            id: tc["id"].as_str().unwrap_or_default().into(),
            name: tc["function"]["name"].as_str().unwrap_or_default().into(),
        });
        out.push(Event::ToolCallArgs { index, delta: tc["function"]["arguments"].as_str().unwrap_or("{}").into() });
    }
    if v["usage"].is_object() {
        out.push(Event::Usage(openai_usage(&v["usage"])));
    }
    if let Some(t) = v["timings"].as_object() {
        out.push(Event::Timings {
            prompt_tps: t.get("prompt_per_second").and_then(Value::as_f64),
            output_tps: t.get("predicted_per_second").and_then(Value::as_f64),
        });
    }
    out.push(Event::Finish(openai_finish(v["choices"][0]["finish_reason"].as_str().unwrap_or("stop"))));
    out
}

fn anthropic_event(v: &Value) -> Result<Vec<Event>, UpstreamError> {
    let index = v["index"].as_u64().unwrap_or(0) as usize;
    Ok(match v["type"].as_str().unwrap_or_default() {
        "message_start" => vec![Event::Usage(anthropic_usage(&v["message"]["usage"]))],
        "content_block_start" => {
            let b = &v["content_block"];
            match b["type"].as_str() {
                Some("tool_use") => vec![Event::ToolCallStart {
                    index,
                    id: b["id"].as_str().unwrap_or_default().into(),
                    name: b["name"].as_str().unwrap_or_default().into(),
                }],
                Some("text") => b["text"].as_str().filter(|s| !s.is_empty()).map(|t| Event::Text(t.into())).into_iter().collect(),
                _ => vec![],
            }
        }
        "content_block_delta" => {
            let d = &v["delta"];
            match d["type"].as_str() {
                Some("text_delta") => vec![Event::Text(d["text"].as_str().unwrap_or_default().into())],
                Some("thinking_delta") => vec![Event::Reasoning(d["thinking"].as_str().unwrap_or_default().into())],
                Some("input_json_delta") => {
                    vec![Event::ToolCallArgs { index, delta: d["partial_json"].as_str().unwrap_or_default().into() }]
                }
                _ => vec![],
            }
        }
        "message_delta" => {
            let mut out = vec![];
            if v["usage"].is_object() {
                // message_delta carries cumulative output tokens only.
                out.push(Event::Usage(Usage { output_tokens: int(&v["usage"]["output_tokens"]), ..Default::default() }));
            }
            if let Some(r) = v["delta"]["stop_reason"].as_str() {
                out.push(Event::Finish(anthropic_finish(r)));
            }
            out
        }
        "error" => return Err(classify(529, &v.to_string())),
        _ => vec![],
    })
}

fn anthropic_full(v: &Value) -> Vec<Event> {
    let mut out = vec![];
    for (index, b) in v["content"].as_array().into_iter().flatten().enumerate() {
        match b["type"].as_str() {
            Some("text") => out.push(Event::Text(b["text"].as_str().unwrap_or_default().into())),
            Some("thinking") => out.push(Event::Reasoning(b["thinking"].as_str().unwrap_or_default().into())),
            Some("tool_use") => {
                out.push(Event::ToolCallStart {
                    index,
                    id: b["id"].as_str().unwrap_or_default().into(),
                    name: b["name"].as_str().unwrap_or_default().into(),
                });
                out.push(Event::ToolCallArgs { index, delta: b["input"].to_string() });
            }
            _ => {}
        }
    }
    out.push(Event::Usage(anthropic_usage(&v["usage"])));
    out.push(Event::Finish(anthropic_finish(v["stop_reason"].as_str().unwrap_or("end_turn"))));
    out
}

/// Turns a provider error into plain words (FR-CHT-14) while keeping the upstream detail.
pub fn classify(status: u16, body: &str) -> UpstreamError {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().or_else(|| v["message"].as_str()).map(str::to_string))
        .unwrap_or_else(|| upstream::truncate(body, 300));
    let lower = detail.to_lowercase();
    if status == 429 {
        return UpstreamError::new(429, "rate_limit", format!("The provider is rate limiting requests. Try again in a moment. ({detail})"), true);
    }
    if lower.contains("context") && (lower.contains("length") || lower.contains("window") || lower.contains("exceed"))
        || lower.contains("too long")
        || lower.contains("prompt is too long")
        || lower.contains("maximum context")
    {
        return UpstreamError::new(400, "context_length", "This conversation is too long for the model's context window. Start a new chat or remove some messages.", false);
    }
    if status == 401 || status == 403 {
        return UpstreamError::new(503, "auth", "The provider rejected the organization's API key. An admin needs to fix the provider key.", false);
    }
    if status >= 500 {
        return UpstreamError::new(503, "unavailable", format!("The provider is having problems (HTTP {status}). Try again shortly. ({detail})"), true);
    }
    UpstreamError::new(if status == 404 { 404 } else { 400 }, "bad_request", format!("The provider rejected the request (HTTP {status}): {detail}"), false)
}

fn network_error(e: &reqwest::Error) -> UpstreamError {
    if e.is_timeout() {
        UpstreamError::new(504, "timeout", "The provider did not respond in time.", true)
    } else {
        UpstreamError::new(502, "unavailable", upstream::network_message(e), true)
    }
}

// ---------- sending ----------

/// Opens a response stream, retrying 429/5xx/network errors with backoff and failing over across keys.
/// Retries only happen before the first byte is returned to the caller.
pub async fn open(state: &AppState, p: &Provider, req: &ChatRequest, streaming: bool) -> Result<EventStream, UpstreamError> {
    upstream::guard_url(state, &p.base_url)
        .await
        .map_err(|e| UpstreamError::new(502, "unavailable", e.message, false))?;
    let mut keys = upstream::ordered_keys(state, p).await.map_err(|e| UpstreamError::new(500, "error", e.message, false))?;
    if keys.is_empty() {
        keys.push(upstream::ProviderKey { id: uuid::Uuid::nil(), secret: String::new() });
    }
    let mut req = req.clone();
    req.stream = streaming;
    let anthropic = p.api_format == "anthropic";
    let (url, body) = if anthropic {
        (upstream::url(p, "messages"), anthropic_body(&req))
    } else {
        (upstream::url(p, "chat/completions"), openai_body(&req, p))
    };
    let mut last_err = UpstreamError::new(503, "unavailable", "No provider key is available", false);
    for key in &keys {
        for attempt in 0..=p.max_retries.max(0) {
            let res = upstream::authed(p, state.http.post(&url), &key.secret).json(&body).send().await;
            let res = match res {
                Ok(r) => r,
                Err(e) => {
                    last_err = network_error(&e);
                    if attempt < p.max_retries {
                        tokio::time::sleep(backoff(attempt, None)).await;
                    }
                    continue;
                }
            };
            let status = res.status();
            if status.is_success() {
                if !key.id.is_nil() {
                    upstream::mark_key(state, key.id, true, None).await;
                }
                return Ok(if streaming { sse_events(res, anthropic) } else { full_events(res, anthropic).await });
            }
            let retry_after = res.headers().get("retry-after").and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
            let text = res.text().await.unwrap_or_default();
            if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                if !key.id.is_nil() {
                    upstream::mark_key(state, key.id, false, Some(format!("HTTP {status}: {}", upstream::truncate(&text, 300)))).await;
                }
                last_err = classify(status.as_u16(), &text);
                break;
            }
            last_err = classify(status.as_u16(), &text);
            let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            if !retryable {
                return Err(last_err);
            }
            if attempt < p.max_retries {
                tokio::time::sleep(backoff(attempt, retry_after)).await;
            }
        }
    }
    Err(last_err)
}

fn backoff(attempt: i32, retry_after: Option<u64>) -> Duration {
    retry_after.map_or_else(|| Duration::from_millis(500 * 2u64.pow(attempt as u32)), |s| Duration::from_secs(s.min(10)))
}

fn sse_events(res: reqwest::Response, anthropic: bool) -> EventStream {
    let s = res.bytes_stream().eventsource().flat_map(move |item| {
        let events: Vec<Result<Event, UpstreamError>> = match item {
            Err(e) => vec![Err(UpstreamError::new(502, "unavailable", format!("Stream interrupted: {e}"), false))],
            Ok(ev) if ev.data.trim() == "[DONE]" || ev.data.is_empty() => vec![],
            Ok(ev) => match serde_json::from_str::<Value>(&ev.data) {
                Err(_) => vec![],
                Ok(v) if !anthropic && v["error"].is_object() => vec![Err(classify(502, &ev.data))],
                Ok(v) if anthropic => match anthropic_event(&v) {
                    Ok(evs) => evs.into_iter().map(Ok).collect(),
                    Err(e) => vec![Err(e)],
                },
                Ok(v) => openai_chunk(&v).into_iter().map(Ok).collect(),
            },
        };
        stream::iter(events)
    });
    Box::pin(s)
}

async fn full_events(res: reqwest::Response, anthropic: bool) -> EventStream {
    let events: Vec<Result<Event, UpstreamError>> = match res.json::<Value>().await {
        Ok(v) => (if anthropic { anthropic_full(&v) } else { openai_full(&v) }).into_iter().map(Ok).collect(),
        Err(e) => vec![Err(UpstreamError::new(502, "error", format!("Unexpected response: {e}"), false))],
    };
    Box::pin(stream::iter(events))
}

// ---------- accumulation ----------

/// Folds an event stream into the final assistant turn. Used by non-streaming callers and the tool loop.
#[derive(Default, Debug)]
pub struct Accumulated {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub finish: Option<String>,
    pub timings: Option<(Option<f64>, Option<f64>)>,
}

impl Accumulated {
    pub fn push(&mut self, ev: &Event) {
        match ev {
            Event::Text(t) => self.text.push_str(t),
            Event::Reasoning(r) => self.reasoning.push_str(r),
            Event::ToolCallStart { index, id, name } => {
                while self.tool_calls.len() <= *index {
                    self.tool_calls.push(ToolCall { id: String::new(), name: String::new(), arguments: String::new() });
                }
                self.tool_calls[*index].id = id.clone();
                self.tool_calls[*index].name = name.clone();
            }
            Event::ToolCallArgs { index, delta } => {
                if let Some(tc) = self.tool_calls.get_mut(*index) {
                    tc.arguments.push_str(delta);
                }
            }
            Event::Usage(u) => self.usage.merge(u.clone()),
            Event::Finish(f) => self.finish = Some(f.clone()),
            Event::Timings { prompt_tps, output_tps } => self.timings = Some((*prompt_tps, *output_tps)),
        }
    }

    /// Tool calls with their sparse Anthropic indexes removed.
    pub fn calls(&self) -> Vec<ToolCall> {
        self.tool_calls
            .iter()
            .filter(|t| !t.name.is_empty())
            .map(|t| ToolCall { arguments: if t.arguments.trim().is_empty() { "{}".into() } else { t.arguments.clone() }, ..t.clone() })
            .collect()
    }
}

// ---------- inbound translation (gateway) ----------

fn parse_openai_content(c: &Value) -> Vec<Part> {
    match c {
        Value::String(s) => vec![Part::Text { text: s.clone() }],
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| match p["type"].as_str() {
                Some("text") => Some(Part::Text { text: p["text"].as_str().unwrap_or_default().into() }),
                Some("image_url") => Some(Part::Image {
                    url: p["image_url"]["url"].as_str().or_else(|| p["image_url"].as_str()).unwrap_or_default().into(),
                }),
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

/// OpenAI chat-completions body → internal request. `model` is left for the caller to resolve.
pub fn from_openai(b: &Map<String, Value>) -> Result<ChatRequest, String> {
    let mut req = ChatRequest::default();
    let mut system = vec![];
    for m in b.get("messages").and_then(Value::as_array).ok_or("messages is required")? {
        let role = m["role"].as_str().unwrap_or_default();
        match role {
            "system" | "developer" => system.push(Msg { role: Role::User, content: parse_openai_content(&m["content"]), tool_calls: vec![], tool_call_id: None }.joined_text()),
            "user" => req.messages.push(Msg { role: Role::User, content: parse_openai_content(&m["content"]), tool_calls: vec![], tool_call_id: None }),
            "assistant" => req.messages.push(Msg {
                role: Role::Assistant,
                content: parse_openai_content(&m["content"]),
                tool_calls: m["tool_calls"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|t| ToolCall {
                        id: t["id"].as_str().unwrap_or_default().into(),
                        name: t["function"]["name"].as_str().unwrap_or_default().into(),
                        arguments: t["function"]["arguments"].as_str().unwrap_or("{}").into(),
                    })
                    .collect(),
                tool_call_id: None,
            }),
            "tool" => req.messages.push(Msg {
                role: Role::Tool,
                content: parse_openai_content(&m["content"]),
                tool_calls: vec![],
                tool_call_id: m["tool_call_id"].as_str().map(str::to_string),
            }),
            other => return Err(format!("unsupported message role: {other}")),
        }
    }
    req.system = (!system.is_empty()).then(|| system.join("\n\n"));
    req.temperature = b.get("temperature").and_then(Value::as_f64);
    req.top_p = b.get("top_p").and_then(Value::as_f64);
    req.max_tokens = b.get("max_completion_tokens").or_else(|| b.get("max_tokens")).and_then(Value::as_i64);
    req.stop = match b.get("stop") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect(),
        _ => vec![],
    };
    req.tools = b
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| t["type"] == "function")
        .map(|t| ToolDef {
            name: t["function"]["name"].as_str().unwrap_or_default().into(),
            description: t["function"]["description"].as_str().unwrap_or_default().into(),
            parameters: t["function"].get("parameters").cloned().unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
        })
        .collect();
    req.stream = b.get("stream").and_then(Value::as_bool).unwrap_or(false);
    Ok(req)
}

fn anthropic_text(c: &Value) -> String {
    match c {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Anthropic messages body → internal request.
pub fn from_anthropic(b: &Map<String, Value>) -> Result<ChatRequest, String> {
    let mut req = ChatRequest { system: b.get("system").map(anthropic_text).filter(|s| !s.is_empty()), ..Default::default() };
    for m in b.get("messages").and_then(Value::as_array).ok_or("messages is required")? {
        let assistant = m["role"] == "assistant";
        let blocks: Vec<Value> = match &m["content"] {
            Value::String(s) => vec![json!({ "type": "text", "text": s })],
            Value::Array(a) => a.clone(),
            _ => vec![],
        };
        let mut msg = Msg { role: if assistant { Role::Assistant } else { Role::User }, content: vec![], tool_calls: vec![], tool_call_id: None };
        for blk in blocks {
            match blk["type"].as_str() {
                Some("text") => msg.content.push(Part::Text { text: blk["text"].as_str().unwrap_or_default().into() }),
                Some("image") => {
                    let s = &blk["source"];
                    let url = if s["type"] == "base64" {
                        format!("data:{};base64,{}", s["media_type"].as_str().unwrap_or("image/png"), s["data"].as_str().unwrap_or_default())
                    } else {
                        s["url"].as_str().unwrap_or_default().into()
                    };
                    msg.content.push(Part::Image { url });
                }
                Some("tool_use") => msg.tool_calls.push(ToolCall {
                    id: blk["id"].as_str().unwrap_or_default().into(),
                    name: blk["name"].as_str().unwrap_or_default().into(),
                    arguments: blk["input"].to_string(),
                }),
                Some("tool_result") => {
                    // Tool results become their own tool turns, in order, before any remaining user content.
                    if !msg.content.is_empty() {
                        req.messages.push(std::mem::replace(&mut msg, Msg { role: Role::User, content: vec![], tool_calls: vec![], tool_call_id: None }));
                    }
                    req.messages.push(Msg {
                        role: Role::Tool,
                        content: vec![Part::Text { text: anthropic_text(&blk["content"]) }],
                        tool_calls: vec![],
                        tool_call_id: blk["tool_use_id"].as_str().map(str::to_string),
                    });
                }
                _ => {}
            }
        }
        if !msg.content.is_empty() || !msg.tool_calls.is_empty() {
            req.messages.push(msg);
        }
    }
    req.temperature = b.get("temperature").and_then(Value::as_f64);
    req.top_p = b.get("top_p").and_then(Value::as_f64);
    req.max_tokens = b.get("max_tokens").and_then(Value::as_i64);
    req.stop = b.get("stop_sequences").and_then(Value::as_array).into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect();
    req.tools = b
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| t.get("input_schema").is_some())
        .map(|t| ToolDef {
            name: t["name"].as_str().unwrap_or_default().into(),
            description: t["description"].as_str().unwrap_or_default().into(),
            parameters: t["input_schema"].clone(),
        })
        .collect();
    req.stream = b.get("stream").and_then(Value::as_bool).unwrap_or(false);
    Ok(req)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(format: &str) -> Provider {
        Provider {
            id: uuid::Uuid::nil(),
            name: "p".into(),
            slug: "p".into(),
            api_format: format.into(),
            base_url: "https://example.com/v1".into(),
            headers: json!({}),
            organization: None,
            project: None,
            key_strategy: "failover".into(),
            timeout_secs: 10,
            max_retries: 0,
            enabled: true,
        }
    }

    #[test]
    fn openai_roundtrip_through_anthropic() {
        let body = json!({
            "model": "x", "stream": true, "max_tokens": 50,
            "messages": [
                { "role": "system", "content": "Be brief" },
                { "role": "user", "content": [{ "type": "text", "text": "hi" }, { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } }] },
                { "role": "assistant", "content": null, "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "web_search", "arguments": "{\"query\":\"q\"}" } }] },
                { "role": "tool", "tool_call_id": "c1", "content": "result" },
                { "role": "tool", "tool_call_id": "c2", "content": "result2" }
            ],
            "tools": [{ "type": "function", "function": { "name": "web_search", "description": "d", "parameters": { "type": "object" } } }]
        });
        let req = from_openai(body.as_object().unwrap()).unwrap();
        assert_eq!(req.system.as_deref(), Some("Be brief"));
        let a = anthropic_body(&req);
        assert_eq!(a["system"], "Be brief");
        assert_eq!(a["messages"][0]["content"][1]["source"]["media_type"], "image/png");
        assert_eq!(a["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(a["messages"][1]["content"][0]["input"]["query"], "q");
        // Two consecutive tool results merge into one user turn.
        assert_eq!(a["messages"][2]["role"], "user");
        assert_eq!(a["messages"][2]["content"].as_array().unwrap().len(), 2);
        assert_eq!(a["tools"][0]["input_schema"]["type"], "object");

        let back = from_anthropic(a.as_object().unwrap()).unwrap();
        let o = openai_body(&back, &provider("openai"));
        assert_eq!(o["messages"][0]["role"], "system");
        assert_eq!(o["messages"][2]["tool_calls"][0]["function"]["name"], "web_search");
        assert_eq!(o["messages"][3]["role"], "tool");
        assert_eq!(o["messages"][4]["tool_call_id"], "c2");
        assert_eq!(o["stream_options"]["include_usage"], true);
    }

    #[test]
    fn parses_stream_events_and_usage() {
        let mut acc = Accumulated::default();
        for ev in openai_chunk(&json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": "c1", "function": { "name": "f", "arguments": "{\"a\"" } }] } }] }))
            .into_iter()
            .chain(openai_chunk(&json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "function": { "arguments": ":1}" } }] }, "finish_reason": "tool_calls" }] })))
            .chain(openai_chunk(&json!({ "choices": [], "usage": { "prompt_tokens": 10, "completion_tokens": 3, "prompt_tokens_details": { "cached_tokens": 4 } } })))
        {
            acc.push(&ev);
        }
        assert_eq!(acc.calls()[0].arguments, "{\"a\":1}");
        assert_eq!(acc.finish.as_deref(), Some("tool_calls"));
        assert_eq!(acc.usage.cached_tokens, Some(4));

        let mut acc = Accumulated::default();
        for v in [
            json!({ "type": "message_start", "message": { "usage": { "input_tokens": 5, "cache_read_input_tokens": 10, "output_tokens": 1 } } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "thinking_delta", "thinking": "hmm" } }),
            json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "text_delta", "text": "Hi" } }),
            json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" }, "usage": { "output_tokens": 7 } }),
        ] {
            for ev in anthropic_event(&v).unwrap() {
                acc.push(&ev);
            }
        }
        assert_eq!((acc.text.as_str(), acc.reasoning.as_str()), ("Hi", "hmm"));
        assert_eq!(acc.usage.input_tokens, Some(15));
        assert_eq!(acc.usage.output_tokens, Some(7));
        assert_eq!(acc.usage.cached_tokens, Some(10));
    }

    #[test]
    fn classifies_errors() {
        assert_eq!(classify(429, "{}").kind, "rate_limit");
        assert_eq!(classify(400, r#"{"error":{"message":"This model's maximum context length is 8192 tokens"}}"#).kind, "context_length");
        assert_eq!(classify(529, "overloaded").kind, "unavailable");
    }
}
