//! The unified API: one platform key reaches every permitted model through an OpenAI-compatible and an
//! Anthropic-compatible endpoint, whatever format the upstream provider speaks.

use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    net::SocketAddr,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, post},
};
use chrono::Utc;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::{
    access::{self, ModelRef},
    llm::{self, Accumulated, Event, EventStream, Usage},
    settings,
    state::AppState,
    usage::{Meter, UsageRow},
};

#[derive(Clone, Copy, PartialEq)]
enum Fmt {
    OpenAi,
    Anthropic,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/models", get(openai_models))
        .route("/api/v1/chat/completions", post(openai_chat))
        .route("/api/v1/usage", get(api_usage))
        .route("/api/anthropic/v1/models", get(anthropic_models))
        .route("/api/anthropic/v1/messages", post(anthropic_messages))
}

fn api_error(fmt: Fmt, status: StatusCode, message: &str) -> Response {
    let s = status.as_u16();
    let body = match fmt {
        Fmt::OpenAi => {
            let kind = match s {
                401 => "authentication_error",
                403 => "permission_error",
                404 => "not_found_error",
                429 => "rate_limit_error",
                400..=499 => "invalid_request_error",
                _ => "api_error",
            };
            json!({ "error": { "message": message, "type": kind, "param": null, "code": null } })
        }
        Fmt::Anthropic => {
            let kind = match s {
                401 => "authentication_error",
                403 => "permission_error",
                404 => "not_found_error",
                429 => "rate_limit_error",
                413 => "request_too_large",
                529 | 503 => "overloaded_error",
                400..=499 => "invalid_request_error",
                _ => "api_error",
            };
            json!({ "type": "error", "error": { "type": kind, "message": message } })
        }
    };
    (status, Json(body)).into_response()
}

struct Caller {
    user_id: Uuid,
    key_id: Uuid,
    model_ids: Option<Vec<Uuid>>,
}

static RATE: LazyLock<Mutex<HashMap<Uuid, (u32, Instant)>>> = LazyLock::new(Default::default);

async fn authenticate(state: &AppState, headers: &HeaderMap, peer: SocketAddr, fmt: Fmt) -> Result<Caller, Response> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()))
        .map(str::trim)
        .ok_or_else(|| api_error(fmt, StatusCode::UNAUTHORIZED, "Missing API key. Send it as a Bearer token or x-api-key."))?;
    let row: Option<(Uuid, Uuid, Option<Vec<Uuid>>, Option<i32>, Option<i64>)> = sqlx::query_as(
        "select k.id, k.user_id, k.model_ids, k.rate_limit_rpm, k.token_quota_month
         from api_keys k join users u on u.id = k.user_id
         where k.key_hash = $1 and k.revoked_at is null and (k.expires_at is null or k.expires_at > now()) and u.status = 'active'",
    )
    .bind(state.secrets.api_key_hash(token))
    .fetch_optional(&state.db)
    .await
    .map_err(|e| api_error(fmt, StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let (key_id, user_id, model_ids, rpm, quota) =
        row.ok_or_else(|| api_error(fmt, StatusCode::UNAUTHORIZED, "Invalid, expired or revoked API key"))?;
    let internal = |e: crate::error::AppError| api_error(fmt, StatusCode::INTERNAL_SERVER_ERROR, &e.message);
    if !access::can_use_api_keys(state, user_id).await.map_err(internal)? {
        return Err(api_error(fmt, StatusCode::FORBIDDEN, "API access is disabled for this account"));
    }
    let max_rpm = settings::get(state).await.map_err(internal)?.max_api_key_rpm;
    let limit = match (rpm, max_rpm) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    if let Some(limit) = limit {
        // ponytail: fixed one-minute window in memory, per node; use governor/Valkey for multi-node.
        let mut m = RATE.lock().unwrap();
        let e = m.entry(key_id).or_insert((0, Instant::now()));
        if e.1.elapsed() > Duration::from_secs(60) {
            *e = (0, Instant::now());
        }
        e.0 += 1;
        if e.0 > limit.max(0) as u32 {
            return Err(api_error(fmt, StatusCode::TOO_MANY_REQUESTS, &format!("Rate limit of {limit} requests per minute exceeded")));
        }
    }
    if let Some(quota) = quota {
        let used: i64 = sqlx::query_scalar(
            "select coalesce(sum(coalesce(input_tokens, 0) + coalesce(output_tokens, 0)), 0)::bigint from usage_log
             where api_key_id = $1 and created_at >= date_trunc('month', now())",
        )
        .bind(key_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| api_error(fmt, StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        if used >= quota {
            return Err(api_error(fmt, StatusCode::TOO_MANY_REQUESTS, "This key's monthly token quota is used up"));
        }
    }
    let ip = state.client_ip(headers, Some(peer));
    let db = state.db.clone();
    tokio::spawn(async move {
        let _ = sqlx::query(
            "update api_keys set last_used_at = now(), last_used_ip = $2
             where id = $1 and (last_used_at is null or last_used_at < now() - interval '1 minute' or last_used_ip is distinct from $2)",
        )
        .bind(key_id)
        .bind(ip)
        .execute(&db)
        .await;
    });
    Ok(Caller { user_id, key_id, model_ids })
}

async fn list_models(state: &AppState, headers: &HeaderMap, peer: SocketAddr, fmt: Fmt) -> Response {
    let caller = match authenticate(state, headers, peer, fmt).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let models = match access::accessible_models(state, caller.user_id).await {
        Ok(m) => m,
        Err(e) => return api_error(fmt, e.status, &e.message),
    };
    let models = models.into_iter().filter(|m| caller.model_ids.as_ref().is_none_or(|ids| ids.contains(&m.id)));
    match fmt {
        Fmt::OpenAi => Json(json!({
            "object": "list",
            "data": models.map(|m| json!({ "id": m.name, "object": "model", "created": 0, "owned_by": m.provider_name })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Fmt::Anthropic => {
            let data: Vec<Value> = models
                .map(|m| json!({ "type": "model", "id": m.name, "display_name": m.display_name, "created_at": "1970-01-01T00:00:00Z" }))
                .collect();
            Json(json!({
                "data": data, "has_more": false,
                "first_id": data.first().map(|d| d["id"].clone()), "last_id": data.last().map(|d| d["id"].clone()),
            }))
            .into_response()
        }
    }
}

async fn openai_models(State(state): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    list_models(&state, &headers, peer, Fmt::OpenAi).await
}

async fn anthropic_models(State(state): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    list_models(&state, &headers, peer, Fmt::Anthropic).await
}

async fn openai_chat(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    complete(state, headers, peer, Fmt::OpenAi, body).await
}

async fn anthropic_messages(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    complete(state, headers, peer, Fmt::Anthropic, body).await
}

async fn complete(state: AppState, headers: HeaderMap, peer: SocketAddr, fmt: Fmt, body: axum::body::Bytes) -> Response {
    let caller = match authenticate(&state, &headers, peer, fmt).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let body: Map<String, Value> = match serde_json::from_slice(&body) {
        Ok(Value::Object(m)) => m,
        _ => return api_error(fmt, StatusCode::BAD_REQUEST, "Request body must be a JSON object"),
    };
    let Some(model_name) = body.get("model").and_then(Value::as_str).map(str::to_string) else {
        return api_error(fmt, StatusCode::BAD_REQUEST, "model is required");
    };
    let parsed = match fmt {
        Fmt::OpenAi => llm::from_openai(&body),
        Fmt::Anthropic => llm::from_anthropic(&body),
    };
    let mut req = match parsed {
        Ok(r) => r,
        Err(e) => return api_error(fmt, StatusCode::BAD_REQUEST, &e),
    };
    let resolved = match access::resolve(&state, caller.user_id, ModelRef::Name(&model_name)).await {
        Ok(r) => r,
        Err(e) => return api_error(fmt, e.status, &e.message),
    };
    if caller.model_ids.as_ref().is_some_and(|ids| !ids.contains(&resolved.model.id)) {
        return api_error(fmt, StatusCode::FORBIDDEN, "This API key is not allowed to use this model");
    }
    let (model, provider) = (resolved.model, resolved.provider);
    req.model = model.upstream_id.clone();
    req.max_tokens = req.max_tokens.or(model.max_output_tokens.map(i64::from));
    let stream = req.stream;
    let include_usage = body.get("stream_options").and_then(|o| o.get("include_usage")).and_then(Value::as_bool).unwrap_or(false);
    let log_content = settings::get(&state).await.map(|s| s.log_content).unwrap_or(false);

    let log = LogCtx {
        state: state.clone(),
        user_id: caller.user_id,
        key_id: caller.key_id,
        model_id: model.id,
        model_name: model.name.clone(),
        provider_id: provider.id,
        provider_name: provider.name.clone(),
        price: (model.price_input_per_m, model.price_output_per_m),
        request: log_content.then(|| Value::Object(body.clone())),
    };
    let meter = Meter::start();
    let upstream = match llm::open(&state, &provider, &req, model.supports_streaming).await {
        Ok(s) => s,
        Err(e) => {
            log.write(&meter, &Accumulated::default(), e.status as i32, Some(e.message.clone()));
            return api_error(fmt, StatusCode::from_u16(e.status).unwrap_or(StatusCode::BAD_GATEWAY), &e.message);
        }
    };
    let mut hdrs = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&provider.name) {
        hdrs.insert("x-braid-provider", v);
    }
    if let Ok(v) = HeaderValue::from_str(&model.name) {
        hdrs.insert("x-braid-model", v);
    }
    let tr = Translator::new(fmt, &model_name, include_usage);
    if stream {
        let st = StreamState { upstream, tr, meter, acc: Accumulated::default(), queue: VecDeque::new(), ended: false, error: None, log: Some(log) };
        let s = futures::stream::unfold(st, |mut st| async move {
            loop {
                if let Some(e) = st.queue.pop_front() {
                    return Some((Ok::<_, Infallible>(e), st));
                }
                if st.ended {
                    return None;
                }
                match st.upstream.next().await {
                    Some(Ok(ev)) => {
                        if matches!(ev, Event::Text(_) | Event::Reasoning(_) | Event::ToolCallStart { .. } | Event::ToolCallArgs { .. }) {
                            st.meter.tick();
                        }
                        st.acc.push(&ev);
                        let out = st.tr.on_event(&ev);
                        st.queue.extend(out);
                    }
                    Some(Err(e)) => {
                        st.queue.extend(st.tr.error(&e.message));
                        st.error = Some(e);
                        st.ended = true;
                    }
                    None => {
                        let out = st.tr.finish(&st.acc);
                        st.queue.extend(out);
                        st.ended = true;
                    }
                }
            }
        });
        return (hdrs, Sse::new(s).keep_alive(KeepAlive::default())).into_response();
    }
    let mut upstream = upstream;
    let mut meter = meter;
    let mut acc = Accumulated::default();
    while let Some(ev) = upstream.next().await {
        match ev {
            Ok(ev) => {
                meter.tick();
                acc.push(&ev);
            }
            Err(e) => {
                log.write(&meter, &acc, e.status as i32, Some(e.message.clone()));
                return api_error(fmt, StatusCode::from_u16(e.status).unwrap_or(StatusCode::BAD_GATEWAY), &e.message);
            }
        }
    }
    log.write(&meter, &acc, 200, None);
    (hdrs, Json(tr.full_response(&acc))).into_response()
}

struct LogCtx {
    state: AppState,
    user_id: Uuid,
    key_id: Uuid,
    model_id: Uuid,
    model_name: String,
    provider_id: Uuid,
    provider_name: String,
    price: (Option<f64>, Option<f64>),
    request: Option<Value>,
}

impl LogCtx {
    fn write(&self, meter: &Meter, acc: &Accumulated, status: i32, error: Option<String>) {
        let m = meter.finish(&acc.usage, acc.timings, self.price.0, self.price.1);
        let row = UsageRow {
            source: "api",
            user_id: Some(self.user_id),
            api_key_id: Some(self.key_id),
            model_id: Some(self.model_id),
            provider_id: Some(self.provider_id),
            model_name: Some(self.model_name.clone()),
            provider_name: Some(self.provider_name.clone()),
            status,
            error,
            content: self.request.as_ref().map(|r| json!({ "request": r, "response": acc.text, "reasoning": acc.reasoning })),
            ..Default::default()
        }
        .with_metrics(&m);
        if self.state.usage.try_send(row).is_err() {
            tracing::warn!("usage channel full; dropped a usage row");
        }
    }
}

struct StreamState {
    upstream: EventStream,
    tr: Translator,
    meter: Meter,
    acc: Accumulated,
    queue: VecDeque<SseEvent>,
    ended: bool,
    error: Option<llm::UpstreamError>,
    log: Option<LogCtx>,
}

/// Logs on drop so client disconnects (status 499) are recorded too.
impl Drop for StreamState {
    fn drop(&mut self) {
        if let Some(log) = self.log.take() {
            let (status, err) = match (&self.error, self.ended) {
                (Some(e), _) => (e.status as i32, Some(e.message.clone())),
                (None, true) => (200, None),
                (None, false) => (499, Some("Client disconnected".into())),
            };
            log.write(&self.meter, &self.acc, status, err);
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Block {
    Text,
    Thinking,
    Tool(usize),
}

/// Re-encodes provider-neutral events in the caller's API format.
struct Translator {
    fmt: Fmt,
    id: String,
    model: String,
    created: i64,
    include_usage: bool,
    sent_role: bool,
    // OpenAI: upstream tool index -> sequential index
    tool_index: HashMap<usize, usize>,
    // Anthropic block state
    started: bool,
    open: Option<(Block, usize)>,
    next_block: usize,
    finish: Option<String>,
    usage: Usage,
}

fn anthropic_stop(f: &str) -> &'static str {
    match f {
        "length" => "max_tokens",
        "tool_calls" => "tool_use",
        "content_filter" => "refusal",
        _ => "end_turn",
    }
}

fn anthropic_usage(u: &Usage) -> Value {
    let cached = u.cached_tokens.unwrap_or(0);
    let write = u.cache_write_tokens.unwrap_or(0);
    json!({
        "input_tokens": u.input_tokens.map(|i| (i - cached - write).max(0)).unwrap_or(0),
        "output_tokens": u.output_tokens.unwrap_or(0),
        "cache_read_input_tokens": u.cached_tokens,
        "cache_creation_input_tokens": u.cache_write_tokens,
    })
}

fn openai_usage(u: &Usage) -> Value {
    let mut v = json!({
        "prompt_tokens": u.input_tokens.unwrap_or(0),
        "completion_tokens": u.output_tokens.unwrap_or(0),
        "total_tokens": u.input_tokens.unwrap_or(0) + u.output_tokens.unwrap_or(0),
    });
    if let Some(c) = u.cached_tokens {
        v["prompt_tokens_details"] = json!({ "cached_tokens": c });
    }
    if let Some(r) = u.reasoning_tokens {
        v["completion_tokens_details"] = json!({ "reasoning_tokens": r });
    }
    v
}

impl Translator {
    fn new(fmt: Fmt, model: &str, include_usage: bool) -> Self {
        let id = match fmt {
            Fmt::OpenAi => format!("chatcmpl-{}", Uuid::new_v4().simple()),
            Fmt::Anthropic => format!("msg_{}", Uuid::new_v4().simple()),
        };
        Self {
            fmt,
            id,
            model: model.into(),
            created: Utc::now().timestamp(),
            include_usage,
            sent_role: false,
            tool_index: HashMap::new(),
            started: false,
            open: None,
            next_block: 0,
            finish: None,
            usage: Usage::default(),
        }
    }

    fn chunk(&mut self, delta: Value, finish: Option<&str>) -> SseEvent {
        let mut delta = delta;
        if !self.sent_role {
            delta["role"] = json!("assistant");
            self.sent_role = true;
        }
        SseEvent::default().data(
            json!({
                "id": self.id, "object": "chat.completion.chunk", "created": self.created, "model": self.model,
                "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
            })
            .to_string(),
        )
    }

    fn a(&self, kind: &str, data: Value) -> SseEvent {
        SseEvent::default().event(kind).data(data.to_string())
    }

    fn a_start(&mut self, out: &mut Vec<SseEvent>) {
        if !self.started {
            self.started = true;
            out.push(self.a("message_start", json!({ "type": "message_start", "message": {
                "id": self.id, "type": "message", "role": "assistant", "model": self.model, "content": [],
                "stop_reason": null, "stop_sequence": null, "usage": anthropic_usage(&self.usage),
            }})));
        }
    }

    fn a_open(&mut self, out: &mut Vec<SseEvent>, block: Block, start: Value) -> usize {
        if let Some((b, i)) = self.open {
            if b == block {
                return i;
            }
            out.push(self.a("content_block_stop", json!({ "type": "content_block_stop", "index": i })));
        }
        let i = self.next_block;
        self.next_block += 1;
        self.open = Some((block, i));
        out.push(self.a("content_block_start", json!({ "type": "content_block_start", "index": i, "content_block": start })));
        i
    }

    fn on_event(&mut self, ev: &Event) -> Vec<SseEvent> {
        let mut out = vec![];
        match self.fmt {
            Fmt::OpenAi => match ev {
                Event::Text(t) => out.push(self.chunk(json!({ "content": t }), None)),
                Event::Reasoning(r) => out.push(self.chunk(json!({ "reasoning_content": r }), None)),
                Event::ToolCallStart { index, id, name } => {
                    let n = self.tool_index.len();
                    let i = *self.tool_index.entry(*index).or_insert(n);
                    out.push(self.chunk(
                        json!({ "tool_calls": [{ "index": i, "id": id, "type": "function", "function": { "name": name, "arguments": "" } }] }),
                        None,
                    ));
                }
                Event::ToolCallArgs { index, delta } => {
                    let n = self.tool_index.len();
                    let i = *self.tool_index.entry(*index).or_insert(n);
                    out.push(self.chunk(json!({ "tool_calls": [{ "index": i, "function": { "arguments": delta } }] }), None));
                }
                Event::Finish(f) => {
                    self.finish = Some(f.clone());
                    out.push(self.chunk(json!({}), Some(f)));
                }
                Event::Usage(u) => merge(&mut self.usage, u),
                Event::Timings { .. } => {}
            },
            Fmt::Anthropic => {
                if let Event::Usage(u) = ev {
                    merge(&mut self.usage, u);
                    return out;
                }
                self.a_start(&mut out);
                match ev {
                    Event::Text(t) => {
                        let i = self.a_open(&mut out, Block::Text, json!({ "type": "text", "text": "" }));
                        out.push(self.a("content_block_delta", json!({ "type": "content_block_delta", "index": i, "delta": { "type": "text_delta", "text": t } })));
                    }
                    Event::Reasoning(r) => {
                        let i = self.a_open(&mut out, Block::Thinking, json!({ "type": "thinking", "thinking": "" }));
                        out.push(self.a("content_block_delta", json!({ "type": "content_block_delta", "index": i, "delta": { "type": "thinking_delta", "thinking": r } })));
                    }
                    Event::ToolCallStart { index, id, name } => {
                        // Force a fresh block even if another tool block is open.
                        if let Some((b, i)) = self.open.take() {
                            let _ = b;
                            out.push(self.a("content_block_stop", json!({ "type": "content_block_stop", "index": i })));
                        }
                        self.a_open(&mut out, Block::Tool(*index), json!({ "type": "tool_use", "id": id, "name": name, "input": {} }));
                    }
                    Event::ToolCallArgs { index, delta } => {
                        if let Some((Block::Tool(t), i)) = self.open {
                            if t == *index {
                                out.push(self.a("content_block_delta", json!({ "type": "content_block_delta", "index": i, "delta": { "type": "input_json_delta", "partial_json": delta } })));
                            }
                        }
                    }
                    Event::Finish(f) => self.finish = Some(f.clone()),
                    _ => {}
                }
            }
        }
        out
    }

    fn finish(&mut self, acc: &Accumulated) -> Vec<SseEvent> {
        let mut out = vec![];
        match self.fmt {
            Fmt::OpenAi => {
                if self.finish.is_none() {
                    let f = acc.finish.clone().unwrap_or_else(|| "stop".into());
                    out.push(self.chunk(json!({}), Some(&f)));
                }
                if self.include_usage {
                    out.push(SseEvent::default().data(
                        json!({ "id": self.id, "object": "chat.completion.chunk", "created": self.created, "model": self.model,
                                "choices": [], "usage": openai_usage(&self.usage) })
                        .to_string(),
                    ));
                }
                out.push(SseEvent::default().data("[DONE]"));
            }
            Fmt::Anthropic => {
                self.a_start(&mut out);
                if let Some((_, i)) = self.open.take() {
                    out.push(self.a("content_block_stop", json!({ "type": "content_block_stop", "index": i })));
                }
                let stop = anthropic_stop(self.finish.as_deref().unwrap_or("stop"));
                out.push(self.a("message_delta", json!({ "type": "message_delta",
                    "delta": { "stop_reason": stop, "stop_sequence": null }, "usage": anthropic_usage(&self.usage) })));
                out.push(self.a("message_stop", json!({ "type": "message_stop" })));
            }
        }
        out
    }

    fn error(&mut self, message: &str) -> Vec<SseEvent> {
        match self.fmt {
            Fmt::OpenAi => vec![
                SseEvent::default().data(json!({ "error": { "message": message, "type": "api_error", "param": null, "code": null } }).to_string()),
                SseEvent::default().data("[DONE]"),
            ],
            Fmt::Anthropic => vec![self.a("error", json!({ "type": "error", "error": { "type": "api_error", "message": message } }))],
        }
    }

    fn full_response(&self, acc: &Accumulated) -> Value {
        let calls = acc.calls();
        let finish = acc.finish.clone().unwrap_or_else(|| "stop".into());
        match self.fmt {
            Fmt::OpenAi => {
                let mut msg = json!({ "role": "assistant", "content": if acc.text.is_empty() && !calls.is_empty() { Value::Null } else { json!(acc.text) } });
                if !acc.reasoning.is_empty() {
                    msg["reasoning_content"] = json!(acc.reasoning);
                }
                if !calls.is_empty() {
                    msg["tool_calls"] = json!(calls.iter().map(|c| json!({
                        "id": c.id, "type": "function", "function": { "name": c.name, "arguments": c.arguments }
                    })).collect::<Vec<_>>());
                }
                json!({
                    "id": self.id, "object": "chat.completion", "created": self.created, "model": self.model,
                    "choices": [{ "index": 0, "message": msg, "finish_reason": finish }],
                    "usage": openai_usage(&acc.usage),
                })
            }
            Fmt::Anthropic => {
                let mut content = vec![];
                if !acc.reasoning.is_empty() {
                    content.push(json!({ "type": "thinking", "thinking": acc.reasoning, "signature": "" }));
                }
                if !acc.text.is_empty() {
                    content.push(json!({ "type": "text", "text": acc.text }));
                }
                for c in &calls {
                    let input: Value = serde_json::from_str(&c.arguments).unwrap_or_else(|_| json!({}));
                    content.push(json!({ "type": "tool_use", "id": c.id, "name": c.name, "input": input }));
                }
                json!({
                    "id": self.id, "type": "message", "role": "assistant", "model": self.model, "content": content,
                    "stop_reason": anthropic_stop(&finish), "stop_sequence": null, "usage": anthropic_usage(&acc.usage),
                })
            }
        }
    }
}

fn merge(a: &mut Usage, b: &Usage) {
    a.input_tokens = b.input_tokens.or(a.input_tokens);
    a.output_tokens = b.output_tokens.or(a.output_tokens);
    a.cached_tokens = b.cached_tokens.or(a.cached_tokens);
    a.cache_write_tokens = b.cache_write_tokens.or(a.cache_write_tokens);
    a.reasoning_tokens = b.reasoning_tokens.or(a.reasoning_tokens);
}

#[derive(Deserialize)]
pub struct UsageQ {
    days: Option<i32>,
}

/// Caller's own usage by day and model (FR-API endpoints table).
async fn api_usage(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<UsageQ>,
) -> Response {
    let caller = match authenticate(&state, &headers, peer, Fmt::OpenAi).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    match user_usage(&state, caller.user_id, q.days.unwrap_or(30)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => api_error(Fmt::OpenAi, e.status, &e.message),
    }
}

pub async fn user_usage(state: &AppState, user_id: Uuid, days: i32) -> crate::error::AppResult<Value> {
    let rows: Vec<(chrono::NaiveDate, Option<String>, i64, Option<i64>, Option<i64>, Option<i64>, Option<f64>)> = sqlx::query_as(
        "select created_at::date as day, model_name, count(*), sum(input_tokens)::bigint, sum(output_tokens)::bigint,
            sum(cached_tokens)::bigint, sum(cost)
         from usage_log where user_id = $1 and source <> 'skill' and created_at >= now() - make_interval(days => $2)
         group by 1, 2 order by 1 desc, 2",
    )
    .bind(user_id)
    .bind(days.clamp(1, 366))
    .fetch_all(&state.db)
    .await?;
    Ok(json!({
        "object": "list",
        "data": rows.iter().map(|r| json!({
            "date": r.0, "model": r.1, "requests": r.2, "input_tokens": r.3, "output_tokens": r.4,
            "cached_tokens": r.5, "cost": r.6,
        })).collect::<Vec<_>>(),
    }))
}
