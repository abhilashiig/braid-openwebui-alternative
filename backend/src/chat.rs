//! Chat history and the streaming reply endpoint, including the server-side tool loop.

use std::convert::Infallible;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::header,
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, patch, post},
};
use chrono::{DateTime, Utc};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    access::{self, ModelRef, Resolved},
    auth::CurrentUser,
    error::{AppError, AppResult},
    llm::{self, Accumulated, ChatRequest, Event, Msg, Part, Role, Usage},
    settings, skills,
    state::AppState,
    usage::{Meter, Metrics, UsageRow},
};

const MAX_ATTACHMENT_BYTES: usize = 10_000_000;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/models", get(my_models))
        .route("/api/me/preferences", patch(set_preferences))
        .route("/api/chats", get(list_chats).post(create_chat))
        .route("/api/chats/{id}", get(get_chat).patch(update_chat).delete(delete_chat))
        .route("/api/chats/{id}/messages", post(send))
        .route("/api/chats/{id}/export", get(export_chat))
}

async fn my_models(State(state): State<AppState>, user: CurrentUser) -> AppResult<Json<Value>> {
    let models = access::accessible_models(&state, user.id).await?;
    let skills = skills::available_for_user(&state, user.id).await?;
    Ok(Json(json!({
        "models": models.iter().map(|m| json!({
            "id": m.id, "name": m.name, "display_name": m.display_name, "description": m.description,
            "provider_name": m.provider_name, "supports_vision": m.supports_vision, "supports_tools": m.supports_tools,
            "supports_reasoning": m.supports_reasoning, "context_window": m.context_window,
            "price_input_per_m": m.price_input_per_m, "price_output_per_m": m.price_output_per_m,
            "forced_skills": m.forced_skills,
        })).collect::<Vec<_>>(),
        "skills": skills,
    })))
}

#[derive(Deserialize)]
struct Prefs {
    default_model_id: Option<Uuid>,
}

async fn set_preferences(State(state): State<AppState>, user: CurrentUser, Json(p): Json<Prefs>) -> AppResult<Json<Value>> {
    if let Some(id) = p.default_model_id {
        access::resolve(&state, user.id, ModelRef::Id(id)).await?;
    }
    sqlx::query("update users set default_model_id = $2 where id = $1").bind(user.id).bind(p.default_model_id).execute(&state.db).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize, sqlx::FromRow)]
struct ChatRow {
    id: Uuid,
    title: String,
    system_prompt: Option<String>,
    temperature: Option<f32>,
    max_tokens: Option<i32>,
    skills: Vec<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const CHAT_COLS: &str = "id, title, system_prompt, temperature, max_tokens, skills, created_at, updated_at";

async fn load_chat(state: &AppState, user_id: Uuid, id: Uuid) -> AppResult<ChatRow> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("select {CHAT_COLS} from chats where id = $1 and user_id = $2")))
        .bind(id)
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Chat"))
}

#[derive(Deserialize)]
struct ListQ {
    q: Option<String>,
}

async fn list_chats(State(state): State<AppState>, user: CurrentUser, Query(q): Query<ListQ>) -> AppResult<Json<Value>> {
    let q = q.q.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let rows: Vec<(Uuid, String, DateTime<Utc>)> = sqlx::query_as(
        "select c.id, c.title, c.updated_at from chats c
         where c.user_id = $1 and ($2::text is null
            or c.title ilike '%' || replace(replace($2, '%', '\\%'), '_', '\\_') || '%'
            or exists(select 1 from messages m where m.chat_id = c.id
                      and to_tsvector('simple', m.content) @@ plainto_tsquery('simple', $2)))
         order by c.updated_at desc limit 300",
    )
    .bind(user.id)
    .bind(q)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(rows.iter().map(|r| json!({ "id": r.0, "title": r.1, "updated_at": r.2 })).collect::<Vec<_>>())))
}

async fn create_chat(State(state): State<AppState>, user: CurrentUser) -> AppResult<Json<ChatRow>> {
    Ok(Json(
        sqlx::query_as(sqlx::AssertSqlSafe(format!("insert into chats (user_id) values ($1) returning {CHAT_COLS}")))
            .bind(user.id)
            .fetch_one(&state.db)
            .await?,
    ))
}

#[derive(Serialize, sqlx::FromRow)]
struct MessageRow {
    id: Uuid,
    role: String,
    content: String,
    attachments: Value,
    reasoning: Option<String>,
    tool_steps: Value,
    sources: Value,
    model_id: Option<Uuid>,
    model_name: Option<String>,
    metrics: Option<Value>,
    error: Option<String>,
    created_at: DateTime<Utc>,
}

const MESSAGE_COLS: &str =
    "id, role, content, attachments, reasoning, tool_steps, sources, model_id, model_name, metrics, error, created_at";

async fn load_messages(state: &AppState, chat_id: Uuid) -> AppResult<Vec<MessageRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {MESSAGE_COLS} from messages where chat_id = $1 order by created_at, role desc"
    )))
    .bind(chat_id)
    .fetch_all(&state.db)
    .await?)
}

async fn totals(state: &AppState, chat_id: Uuid) -> AppResult<Value> {
    let t: (Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<f64>, i64) = sqlx::query_as(
        "select sum(input_tokens)::bigint, sum(output_tokens)::bigint, sum(cached_tokens)::bigint,
            sum(cache_write_tokens)::bigint, sum(reasoning_tokens)::bigint, sum(cost), count(cached_tokens)
         from usage_log where chat_id = $1 and source = 'chat'",
    )
    .bind(chat_id)
    .fetch_one(&state.db)
    .await?;
    Ok(json!({
        "input_tokens": t.0, "output_tokens": t.1, "cached_tokens": t.2, "cache_write_tokens": t.3,
        "reasoning_tokens": t.4, "cost": t.5, "cache_reported": t.6 > 0,
    }))
}

async fn get_chat(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let chat = load_chat(&state, user.id, id).await?;
    let messages = load_messages(&state, id).await?;
    Ok(Json(json!({ "chat": chat, "messages": messages, "totals": totals(&state, id).await? })))
}

#[derive(Deserialize)]
struct ChatPatch {
    title: Option<String>,
    #[serde(default, deserialize_with = "double")]
    system_prompt: Option<Option<String>>,
    #[serde(default, deserialize_with = "double")]
    temperature: Option<Option<f32>>,
    #[serde(default, deserialize_with = "double")]
    max_tokens: Option<Option<i32>>,
    skills: Option<Vec<String>>,
}

fn double<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

async fn update_chat(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(p): Json<ChatPatch>,
) -> AppResult<Json<ChatRow>> {
    load_chat(&state, user.id, id).await?;
    if p.temperature.flatten().is_some_and(|t| !(0.0..=2.0).contains(&t)) {
        return Err(AppError::bad_request("Temperature must be between 0 and 2"));
    }
    if p.max_tokens.flatten().is_some_and(|t| t < 1) {
        return Err(AppError::bad_request("Max tokens must be positive"));
    }
    let title = p.title.map(|t| t.trim().chars().take(200).collect::<String>()).filter(|t| !t.is_empty());
    let row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "update chats set title = coalesce($2, title),
            system_prompt = case when $3 then $4 else system_prompt end,
            temperature = case when $5 then $6 else temperature end,
            max_tokens = case when $7 then $8 else max_tokens end,
            skills = coalesce($9, skills)
         where id = $1 returning {CHAT_COLS}"
    )))
    .bind(id)
    .bind(title)
    .bind(p.system_prompt.is_some())
    .bind(p.system_prompt.flatten().filter(|s| !s.trim().is_empty()))
    .bind(p.temperature.is_some())
    .bind(p.temperature.flatten())
    .bind(p.max_tokens.is_some())
    .bind(p.max_tokens.flatten())
    .bind(p.skills)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(row))
}

async fn delete_chat(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let n = sqlx::query("delete from chats where id = $1 and user_id = $2").bind(id).bind(user.id).execute(&state.db).await?;
    if n.rows_affected() == 0 {
        return Err(AppError::not_found("Chat"));
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ExportQ {
    format: Option<String>,
}

async fn export_chat(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(q): Query<ExportQ>,
) -> AppResult<Response> {
    let chat = load_chat(&state, user.id, id).await?;
    let messages = load_messages(&state, id).await?;
    let safe: String = chat.title.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    if q.format.as_deref() == Some("json") {
        let body = serde_json::to_string_pretty(&json!({ "chat": chat, "messages": messages })).unwrap();
        return Ok((
            [(header::CONTENT_TYPE, "application/json".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{safe}.json\""))],
            body,
        )
            .into_response());
    }
    let mut md = format!("# {}\n\n", chat.title);
    for m in &messages {
        let who = if m.role == "user" { "You".to_string() } else { m.model_name.clone().unwrap_or_else(|| "Assistant".into()) };
        md.push_str(&format!("## {who}\n\n{}\n\n", m.content));
    }
    Ok((
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{safe}.md\""))],
        md,
    )
        .into_response())
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Attachment {
    Image { name: String, url: String },
    File { name: String, text: String },
}

#[derive(Deserialize)]
struct SendReq {
    model_id: Uuid,
    #[serde(default)]
    content: String,
    #[serde(default)]
    attachments: Vec<Attachment>,
    /// Re-run from this user message, replacing it and everything after it.
    edit_message_id: Option<Uuid>,
    /// Replace the last reply without adding a user message.
    #[serde(default)]
    regenerate: bool,
}

fn title_from(text: &str) -> String {
    // ponytail: first line of the first message; an LLM-written title would cost an extra call per chat.
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("New chat").trim();
    if line.chars().count() <= 60 {
        return line.to_string();
    }
    let cut: String = line.chars().take(60).collect();
    format!("{}…", cut.rsplit_once(' ').map_or(cut.as_str(), |(a, _)| a))
}

async fn send(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(chat_id): Path<Uuid>,
    Json(req): Json<SendReq>,
) -> AppResult<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>> {
    let chat = load_chat(&state, user.id, chat_id).await?;
    let resolved = access::resolve(&state, user.id, ModelRef::Id(req.model_id)).await?;
    let size: usize = req
        .attachments
        .iter()
        .map(|a| match a {
            Attachment::Image { url, .. } => url.len(),
            Attachment::File { text, .. } => text.len(),
        })
        .sum();
    if size > MAX_ATTACHMENT_BYTES {
        return Err(AppError::bad_request("Attachments are too large (10 MB max per message)"));
    }
    for a in &req.attachments {
        if let Attachment::Image { url, .. } = a {
            if !(url.starts_with("data:image/") || url.starts_with("https://")) {
                return Err(AppError::bad_request("Images must be uploaded files or https URLs"));
            }
        }
    }

    let mut tx = state.db.begin().await?;
    if let Some(edit_id) = req.edit_message_id {
        let n = sqlx::query(
            "delete from messages where chat_id = $1 and created_at >=
                (select created_at from messages where id = $2 and chat_id = $1 and role = 'user')",
        )
        .bind(chat_id)
        .bind(edit_id)
        .execute(&mut *tx)
        .await?;
        if n.rows_affected() == 0 {
            return Err(AppError::not_found("Message"));
        }
    }
    let mut user_message = None;
    if req.regenerate {
        sqlx::query(
            "delete from messages where chat_id = $1 and role = 'assistant' and created_at >
                coalesce((select max(created_at) from messages where chat_id = $1 and role = 'user'), '-infinity')",
        )
        .bind(chat_id)
        .execute(&mut *tx)
        .await?;
    } else {
        if req.content.trim().is_empty() && req.attachments.is_empty() {
            return Err(AppError::bad_request("Message is empty"));
        }
        let row: MessageRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "insert into messages (chat_id, role, content, attachments) values ($1, 'user', $2, $3) returning {MESSAGE_COLS}"
        )))
        .bind(chat_id)
        .bind(&req.content)
        .bind(serde_json::to_value(&req.attachments).unwrap())
        .fetch_one(&mut *tx)
        .await?;
        user_message = Some(row);
    }
    let mut new_title = None;
    if chat.title == "New chat" {
        if let Some(m) = &user_message {
            let t = title_from(if m.content.trim().is_empty() { "Attachment" } else { &m.content });
            sqlx::query("update chats set title = $2 where id = $1").bind(chat_id).bind(&t).execute(&mut *tx).await?;
            new_title = Some(t);
        }
    }
    tx.commit().await?;

    let history = load_messages(&state, chat_id).await?;
    if history.last().is_none_or(|m| m.role != "user") {
        return Err(AppError::bad_request("Nothing to reply to"));
    }
    let (etx, erx) = mpsc::channel::<SseEvent>(256);
    if let Some(m) = &user_message {
        let _ = etx.try_send(sse("user_message", json!(m)));
    }
    if let Some(t) = new_title {
        let _ = etx.try_send(sse("title", json!({ "title": t })));
    }
    tokio::spawn(generate(state, user, chat, resolved, history, etx));
    let stream = futures::stream::unfold(erx, |mut rx| async move { rx.recv().await.map(|e| (Ok::<_, Infallible>(e), rx)) });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn sse(event: &str, data: Value) -> SseEvent {
    SseEvent::default().event(event).data(data.to_string())
}

fn to_msg(m: &MessageRow, vision: bool) -> Msg {
    let role = if m.role == "user" { Role::User } else { Role::Assistant };
    let mut content = vec![];
    let attachments: Vec<Attachment> = serde_json::from_value(m.attachments.clone()).unwrap_or_default();
    for a in &attachments {
        if let Attachment::File { name, text } = a {
            content.push(Part::Text { text: format!("<file name=\"{name}\">\n{text}\n</file>") });
        }
    }
    if !m.content.is_empty() {
        content.push(Part::Text { text: m.content.clone() });
    }
    for a in attachments {
        if let Attachment::Image { url, name } = a {
            content.push(if vision { Part::Image { url } } else { Part::Text { text: format!("[image {name} omitted: this model cannot see images]") } });
        }
    }
    Msg { role, content, tool_calls: vec![], tool_call_id: None }
}

fn add(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    }
}

async fn generate(
    state: AppState,
    user: CurrentUser,
    chat: ChatRow,
    r: Resolved,
    history: Vec<MessageRow>,
    tx: mpsc::Sender<SseEvent>,
) {
    let Resolved { model, provider } = r;
    let s = settings::get(&state).await.unwrap_or_default();
    let skill_names: Vec<String> = if model.supports_tools {
        let avail = skills::available_for_user(&state, user.id).await.unwrap_or_default();
        let mut wanted: Vec<String> = chat.skills.iter().chain(model.forced_skills.iter()).cloned().collect();
        wanted.sort();
        wanted.dedup();
        wanted.into_iter().filter(|n| avail.contains(n)).collect()
    } else {
        vec![]
    };
    let base = ChatRequest {
        model: model.upstream_id.clone(),
        system: chat.system_prompt.clone().or(model.system_prompt.clone()),
        temperature: chat.temperature.or(model.default_temperature).map(f64::from),
        max_tokens: chat.max_tokens.or(model.max_output_tokens).map(i64::from),
        tools: skills::tool_defs(&skill_names),
        stream: model.supports_streaming,
        ..Default::default()
    };
    let mut msgs: Vec<Msg> = history.iter().map(|m| to_msg(m, model.supports_vision)).collect();
    let assistant_id = Uuid::new_v4();
    let _ = tx.send(sse("start", json!({ "id": assistant_id, "model_id": model.id, "model_name": model.display_name }))).await;

    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_steps: Vec<Value> = vec![];
    let mut sources: Vec<skills::Source> = vec![];
    let mut total = Usage::default();
    let mut total_cost: Option<f64> = None;
    let mut last: Option<Metrics> = None;
    let mut error: Option<llm::UpstreamError> = None;
    let mut stopped = false;
    let max_calls = s.max_tool_calls.max(1) as usize;
    let mut calls_made = 0usize;

    loop {
        let mut req = base.clone();
        req.messages = msgs.clone();
        if calls_made >= max_calls {
            req.tools.clear();
        }
        let mut meter = Meter::start();
        let opened = tokio::select! {
            r = llm::open(&state, &provider, &req, model.supports_streaming) => r,
            _ = tx.closed() => { stopped = true; break; }
        };
        let mut stream = match opened {
            Ok(s) => s,
            Err(e) => {
                let _ = state.usage.try_send(usage_row(&user, &chat, &model.id, &model.name, &provider, e.status as i32, Some(e.message.clone()), None));
                error = Some(e);
                break;
            }
        };
        let mut acc = Accumulated::default();
        let mut need_sep = !content.is_empty();
        loop {
            let ev = tokio::select! {
                ev = stream.next() => ev,
                _ = tx.closed() => { stopped = true; break; }
            };
            match ev {
                None => break,
                Some(Err(e)) => {
                    error = Some(e);
                    break;
                }
                Some(Ok(ev)) => {
                    match &ev {
                        Event::Text(t) => {
                            meter.tick();
                            let t = if need_sep && !t.trim().is_empty() {
                                need_sep = false;
                                format!("\n\n{}", t.trim_start())
                            } else {
                                t.clone()
                            };
                            content.push_str(&t);
                            let _ = tx.send(sse("delta", json!({ "text": t }))).await;
                        }
                        Event::Reasoning(r) => {
                            meter.tick();
                            reasoning.push_str(r);
                            let _ = tx.send(sse("reasoning", json!({ "text": r }))).await;
                        }
                        Event::ToolCallStart { .. } | Event::ToolCallArgs { .. } => meter.tick(),
                        _ => {}
                    }
                    acc.push(&ev);
                }
            }
        }
        let m = meter.finish(&acc.usage, acc.timings, model.price_input_per_m, model.price_output_per_m);
        let status = error.as_ref().map_or(if stopped { 499 } else { 200 }, |e| e.status as i32);
        let _ = state.usage.try_send(
            usage_row(&user, &chat, &model.id, &model.name, &provider, status, error.as_ref().map(|e| e.message.clone()), Some(&m)),
        );
        total.input_tokens = add(total.input_tokens, m.input_tokens);
        total.output_tokens = add(total.output_tokens, m.output_tokens);
        total.cached_tokens = add(total.cached_tokens, m.cached_tokens);
        total.cache_write_tokens = add(total.cache_write_tokens, m.cache_write_tokens);
        total.reasoning_tokens = add(total.reasoning_tokens, m.reasoning_tokens);
        total_cost = match (total_cost, m.cost) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
        };
        last = Some(m);
        let calls = acc.calls();
        if stopped || error.is_some() || calls.is_empty() || calls_made >= max_calls {
            break;
        }
        msgs.push(Msg { role: Role::Assistant, content: vec![Part::Text { text: acc.text.clone() }], tool_calls: calls.clone(), tool_call_id: None });
        for call in calls {
            calls_made += 1;
            let _ = tx.send(sse("tool_call", json!({ "id": call.id, "name": call.name, "arguments": call.arguments }))).await;
            let out = if skill_names.contains(&skills::WEB_SEARCH.to_string()) {
                skills::run(&state, user.id, None, &call.name, &call.arguments).await
            } else {
                skills::SkillOutput { for_model: "Error: tool not available".into(), summary: call.name.clone(), sources: vec![], error: Some("Tool not available".into()) }
            };
            let step = json!({
                "id": call.id, "name": call.name, "arguments": call.arguments, "summary": out.summary,
                "sources": out.sources, "error": out.error,
            });
            let _ = tx.send(sse("tool_result", step.clone())).await;
            tool_steps.push(step);
            for src in out.sources {
                if !sources.iter().any(|x| x.url == src.url) {
                    sources.push(src);
                }
            }
            msgs.push(Msg { role: Role::Tool, content: vec![Part::Text { text: out.for_model }], tool_calls: vec![], tool_call_id: Some(call.id) });
        }
    }

    let mut metrics = last.unwrap_or_default();
    metrics.input_tokens = total.input_tokens;
    metrics.output_tokens = total.output_tokens;
    metrics.cached_tokens = total.cached_tokens;
    metrics.cache_write_tokens = total.cache_write_tokens;
    metrics.reasoning_tokens = total.reasoning_tokens;
    metrics.cost = total_cost;
    let error_text = error.as_ref().map(|e| e.message.clone()).or(stopped.then(|| "Stopped".to_string()));
    let saved: Result<MessageRow, sqlx::Error> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "insert into messages (id, chat_id, role, content, reasoning, tool_steps, sources, model_id, model_name, metrics, error)
         values ($1, $2, 'assistant', $3, $4, $5, $6, $7, $8, $9, $10) returning {MESSAGE_COLS}"
    )))
    .bind(assistant_id)
    .bind(chat.id)
    .bind(&content)
    .bind(Some(&reasoning).filter(|r| !r.is_empty()))
    .bind(json!(tool_steps))
    .bind(json!(sources))
    .bind(model.id)
    .bind(&model.display_name)
    .bind((error.is_none()).then(|| json!(metrics)))
    .bind(error_text)
    .fetch_one(&state.db)
    .await;
    let _ = sqlx::query("update chats set updated_at = now() where id = $1").bind(chat.id).execute(&state.db).await;
    match (saved, error) {
        (Ok(row), None) => {
            let _ = tx.send(sse("done", json!({ "message": row }))).await;
        }
        (Ok(row), Some(e)) => {
            let _ = tx.send(sse("error", json!({ "message": e.message, "kind": e.kind, "retryable": e.retryable, "saved": row }))).await;
        }
        (Err(e), _) => {
            tracing::error!("failed to save assistant message: {e}");
            let _ = tx.send(sse("error", json!({ "message": "The reply could not be saved", "kind": "error", "retryable": true }))).await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn usage_row(
    user: &CurrentUser,
    chat: &ChatRow,
    model_id: &Uuid,
    model_name: &str,
    provider: &crate::upstream::Provider,
    status: i32,
    error: Option<String>,
    m: Option<&Metrics>,
) -> UsageRow {
    let row = UsageRow {
        source: "chat",
        user_id: Some(user.id),
        chat_id: Some(chat.id),
        model_id: Some(*model_id),
        provider_id: Some(provider.id),
        model_name: Some(model_name.into()),
        provider_name: Some(provider.name.clone()),
        status,
        error,
        ..Default::default()
    };
    match m {
        Some(m) => row.with_metrics(m),
        None => row,
    }
}
