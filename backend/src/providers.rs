//! Admin endpoints for providers, provider keys, models and model grants.

use std::collections::HashSet;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    audit,
    auth::Admin,
    crypto,
    error::{AppError, AppResult},
    settings,
    state::AppState,
    upstream::{self, PROVIDER_COLS, Provider},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/providers", get(list_providers).post(create_provider))
        .route("/api/admin/providers/{id}", put(update_provider).delete(delete_provider))
        .route("/api/admin/providers/{id}/keys", post(add_key))
        .route("/api/admin/providers/{id}/test", post(test_provider))
        .route("/api/admin/providers/{id}/upstream-models", get(upstream_models))
        .route("/api/admin/provider-keys/{id}", patch(update_key).delete(delete_key))
        .route("/api/admin/models", get(list_models).post(create_model))
        .route("/api/admin/models/import", post(import_models))
        .route("/api/admin/models/{id}", put(update_model).delete(delete_model))
        .route("/api/admin/models/{id}/grants", get(model_grants).put(set_model_grants))
        .route("/api/admin/models/{id}/impact", get(visibility_impact))
}

#[derive(Serialize, sqlx::FromRow)]
struct KeyView {
    id: Uuid,
    provider_id: Uuid,
    label: String,
    last4: String,
    position: i32,
    enabled: bool,
    healthy: bool,
    last_error: Option<String>,
}

async fn list_providers(State(state): State<AppState>, _: Admin) -> AppResult<Json<Value>> {
    let providers: Vec<Provider> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("select {PROVIDER_COLS} from providers order by name"))).fetch_all(&state.db).await?;
    let keys: Vec<KeyView> = sqlx::query_as(
        "select id, provider_id, label, last4, position, enabled, healthy, last_error from provider_keys order by position, created_at",
    )
    .fetch_all(&state.db)
    .await?;
    let models: Vec<(Uuid, Uuid, String)> =
        sqlx::query_as("select provider_id, id, display_name from models order by display_name").fetch_all(&state.db).await?;
    let out: Vec<Value> = providers
        .into_iter()
        .map(|p| {
            let pid = p.id;
            let mut v = serde_json::to_value(p).unwrap();
            v["keys"] = json!(keys.iter().filter(|k| k.provider_id == pid).collect::<Vec<_>>());
            v["models"] = json!(
                models.iter().filter(|m| m.0 == pid).map(|m| json!({ "id": m.1, "display_name": m.2 })).collect::<Vec<_>>()
            );
            v
        })
        .collect();
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
struct ProviderReq {
    name: String,
    api_format: String,
    base_url: String,
    #[serde(default)]
    headers: serde_json::Map<String, Value>,
    organization: Option<String>,
    project: Option<String>,
    #[serde(default = "default_strategy")]
    key_strategy: String,
    #[serde(default = "default_timeout")]
    timeout_secs: i32,
    #[serde(default = "default_retries")]
    max_retries: i32,
    #[serde(default = "yes")]
    enabled: bool,
    /// Allow-lists the base URL's host when it is a private address (local Ollama, vLLM).
    #[serde(default)]
    allow_private: bool,
    #[serde(default)]
    keys: Vec<NewKey>,
}

fn default_strategy() -> String {
    "failover".into()
}
fn default_timeout() -> i32 {
    120
}
fn default_retries() -> i32 {
    2
}
fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct NewKey {
    label: String,
    key: String,
}

async fn validate_provider(state: &AppState, admin: &Admin, req: &mut ProviderReq) -> AppResult<()> {
    req.name = req.name.trim().to_string();
    req.base_url = req.base_url.trim().trim_end_matches('/').to_string();
    if req.name.is_empty() {
        return Err(AppError::bad_request("Name is required"));
    }
    if !matches!(req.api_format.as_str(), "openai" | "anthropic") {
        return Err(AppError::bad_request("API format must be openai or anthropic"));
    }
    if !matches!(req.key_strategy.as_str(), "failover" | "round_robin") {
        return Err(AppError::bad_request("Key strategy must be failover or round_robin"));
    }
    if !(1..=3600).contains(&req.timeout_secs) || !(0..=10).contains(&req.max_retries) {
        return Err(AppError::bad_request("Timeout must be 1-3600 s and retries 0-10"));
    }
    if req.headers.values().any(|v| !v.is_string()) {
        return Err(AppError::bad_request("Custom header values must be strings"));
    }
    if req.allow_private {
        if let Some(host) = upstream::host_of(&req.base_url) {
            let mut s = settings::get(state).await?;
            if !s.allowed_private_hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
                s.allowed_private_hosts.push(host.clone());
                settings::put(state, &s).await?;
                audit::record(state, &admin.0, "settings.private_host_allowed", "settings", "instance", json!({ "host": host }))
                    .await;
            }
        }
    }
    upstream::guard_url(state, &req.base_url).await
}

fn slugify(name: &str) -> String {
    let s: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "provider".into() } else { s }
}

async fn create_provider(State(state): State<AppState>, admin: Admin, Json(mut req): Json<ProviderReq>) -> AppResult<Json<Value>> {
    validate_provider(&state, &admin, &mut req).await?;
    let base = slugify(&req.name);
    let taken: Vec<String> = sqlx::query_scalar("select slug from providers where slug = $1 or slug like $1 || '-%'")
        .bind(&base)
        .fetch_all(&state.db)
        .await?;
    let slug = (1..).map(|i| if i == 1 { base.clone() } else { format!("{base}-{i}") }).find(|s| !taken.contains(s)).unwrap();
    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "insert into providers (name, slug, api_format, base_url, headers, organization, project, key_strategy, timeout_secs, max_retries, enabled)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) returning id",
    )
    .bind(&req.name)
    .bind(&slug)
    .bind(&req.api_format)
    .bind(&req.base_url)
    .bind(Value::Object(req.headers.clone()))
    .bind(req.organization.as_deref().filter(|s| !s.is_empty()))
    .bind(req.project.as_deref().filter(|s| !s.is_empty()))
    .bind(&req.key_strategy)
    .bind(req.timeout_secs)
    .bind(req.max_retries)
    .bind(req.enabled)
    .fetch_one(&mut *tx)
    .await?;
    for (i, k) in req.keys.iter().enumerate() {
        insert_key(&state, &mut tx, id, k, i as i32).await?;
    }
    tx.commit().await?;
    audit::record(
        &state,
        &admin.0,
        "provider.created",
        "provider",
        id,
        json!({ "name": req.name, "base_url": req.base_url, "api_format": req.api_format, "keys": req.keys.len() }),
    )
    .await;
    Ok(Json(json!({ "id": id, "slug": slug })))
}

async fn insert_key(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    provider_id: Uuid,
    k: &NewKey,
    position: i32,
) -> AppResult<Uuid> {
    let secret = k.key.trim();
    if secret.is_empty() {
        return Err(AppError::bad_request("API key is empty"));
    }
    let label = if k.label.trim().is_empty() { "Default" } else { k.label.trim() };
    Ok(sqlx::query_scalar(
        "insert into provider_keys (provider_id, label, ciphertext, last4, position) values ($1, $2, $3, $4, $5) returning id",
    )
    .bind(provider_id)
    .bind(label)
    .bind(state.secrets.encrypt(secret))
    .bind(crypto::last4(secret))
    .bind(position)
    .fetch_one(tx)
    .await?)
}

async fn update_provider(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(mut req): Json<ProviderReq>,
) -> AppResult<Json<Value>> {
    validate_provider(&state, &admin, &mut req).await?;
    let n = sqlx::query(
        "update providers set name = $2, api_format = $3, base_url = $4, headers = $5, organization = $6, project = $7,
         key_strategy = $8, timeout_secs = $9, max_retries = $10, enabled = $11 where id = $1",
    )
    .bind(id)
    .bind(&req.name)
    .bind(&req.api_format)
    .bind(&req.base_url)
    .bind(Value::Object(req.headers.clone()))
    .bind(req.organization.as_deref().filter(|s| !s.is_empty()))
    .bind(req.project.as_deref().filter(|s| !s.is_empty()))
    .bind(&req.key_strategy)
    .bind(req.timeout_secs)
    .bind(req.max_retries)
    .bind(req.enabled)
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("Provider"));
    }
    audit::record(
        &state,
        &admin.0,
        "provider.updated",
        "provider",
        id,
        json!({ "name": req.name, "base_url": req.base_url, "api_format": req.api_format, "enabled": req.enabled,
                "key_strategy": req.key_strategy, "header_names": req.headers.keys().collect::<Vec<_>>() }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_provider(State(state): State<AppState>, admin: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let name: String = sqlx::query_scalar("delete from providers where id = $1 returning name")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Provider"))?;
    audit::record(&state, &admin.0, "provider.deleted", "provider", id, json!({ "name": name })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn add_key(
    State(state): State<AppState>,
    admin: Admin,
    Path(provider_id): Path<Uuid>,
    Json(k): Json<NewKey>,
) -> AppResult<Json<Value>> {
    let mut tx = state.db.begin().await?;
    let position: i32 = sqlx::query_scalar("select coalesce(max(position) + 1, 0) from provider_keys where provider_id = $1")
        .bind(provider_id)
        .fetch_one(&mut *tx)
        .await?;
    let id = insert_key(&state, &mut tx, provider_id, &k, position).await?;
    tx.commit().await?;
    audit::record(
        &state,
        &admin.0,
        "provider_key.created",
        "provider_key",
        id,
        json!({ "provider_id": provider_id, "label": k.label, "last4": crypto::last4(k.key.trim()) }),
    )
    .await;
    Ok(Json(json!({ "id": id })))
}

#[derive(Deserialize)]
struct KeyPatch {
    label: Option<String>,
    enabled: Option<bool>,
    position: Option<i32>,
    /// Re-enable a key previously marked unhealthy.
    healthy: Option<bool>,
}

async fn update_key(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(p): Json<KeyPatch>,
) -> AppResult<Json<Value>> {
    let n = sqlx::query(
        "update provider_keys set label = coalesce($2, label), enabled = coalesce($3, enabled), position = coalesce($4, position),
         healthy = coalesce($5, healthy), last_error = case when $5 then null else last_error end where id = $1",
    )
    .bind(id)
    .bind(p.label.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(p.enabled)
    .bind(p.position)
    .bind(p.healthy)
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("Key"));
    }
    audit::record(
        &state,
        &admin.0,
        "provider_key.updated",
        "provider_key",
        id,
        json!({ "label": p.label, "enabled": p.enabled, "position": p.position, "healthy": p.healthy }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_key(State(state): State<AppState>, admin: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let row: (Uuid, String, String) =
        sqlx::query_as("delete from provider_keys where id = $1 returning provider_id, label, last4")
            .bind(id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| AppError::not_found("Key"))?;
    audit::record(
        &state,
        &admin.0,
        "provider_key.deleted",
        "provider_key",
        id,
        json!({ "provider_id": row.0, "label": row.1, "last4": row.2 }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

async fn test_provider(State(state): State<AppState>, _: Admin, Path(id): Path<Uuid>) -> AppResult<Json<upstream::TestResult>> {
    let p = upstream::load_provider(&state, id).await?;
    Ok(Json(match upstream::list_models(&state, &p).await {
        Ok(ids) => upstream::TestResult {
            ok: true,
            status: "ok",
            message: format!("Connected. {} models available.", ids.len()),
            model_count: Some(ids.len()),
        },
        Err(e) => e,
    }))
}

/// Upstream model list annotated with what is already registered, plus registered models missing upstream.
async fn upstream_models(State(state): State<AppState>, _: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let p = upstream::load_provider(&state, id).await?;
    let ids = upstream::list_models(&state, &p).await.map_err(|e| AppError::bad_request(e.message))?;
    let registered: Vec<(Uuid, String)> =
        sqlx::query_as("select id, upstream_id from models where provider_id = $1").bind(id).fetch_all(&state.db).await?;
    let upstream_set: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let available: Vec<Value> = ids
        .iter()
        .map(|m| {
            let existing = registered.iter().find(|r| &r.1 == m).map(|r| r.0);
            json!({ "upstream_id": m, "model_id": existing })
        })
        .collect();
    let removed: Vec<Value> = registered
        .iter()
        .filter(|r| !upstream_set.contains(r.1.as_str()))
        .map(|r| json!({ "model_id": r.0, "upstream_id": r.1 }))
        .collect();
    Ok(Json(json!({ "available": available, "removed": removed })))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Model {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub upstream_id: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub visibility: String,
    pub enabled: bool,
    pub context_window: Option<i32>,
    pub max_output_tokens: Option<i32>,
    pub default_temperature: Option<f32>,
    pub system_prompt: Option<String>,
    pub supports_vision: bool,
    pub supports_tools: bool,
    pub supports_reasoning: bool,
    pub supports_streaming: bool,
    pub price_input_per_m: Option<f64>,
    pub price_output_per_m: Option<f64>,
    pub forced_skills: Vec<String>,
}

pub const MODEL_COLS: &str = "m.id, m.provider_id, p.name as provider_name, m.upstream_id, m.name, m.display_name, m.description,
    m.visibility, m.enabled, m.context_window, m.max_output_tokens, m.default_temperature, m.system_prompt,
    m.supports_vision, m.supports_tools, m.supports_reasoning, m.supports_streaming, m.price_input_per_m,
    m.price_output_per_m, m.forced_skills";

async fn list_models(State(state): State<AppState>, _: Admin) -> AppResult<Json<Value>> {
    let models: Vec<Model> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {MODEL_COLS} from models m join providers p on p.id = m.provider_id order by p.name, m.display_name"
    )))
    .fetch_all(&state.db)
    .await?;
    let grant_counts: Vec<(Uuid, i64)> =
        sqlx::query_as("select model_id, count(*) from grants where model_id is not null group by model_id")
            .fetch_all(&state.db)
            .await?;
    let out: Vec<Value> = models
        .into_iter()
        .map(|m| {
            let count = grant_counts.iter().find(|g| g.0 == m.id).map_or(0, |g| g.1);
            let mut v = serde_json::to_value(m).unwrap();
            v["grant_count"] = json!(count);
            v
        })
        .collect();
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
struct ModelReq {
    provider_id: Uuid,
    upstream_id: String,
    name: Option<String>,
    display_name: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default = "private")]
    visibility: String,
    #[serde(default = "yes")]
    enabled: bool,
    context_window: Option<i32>,
    max_output_tokens: Option<i32>,
    default_temperature: Option<f32>,
    system_prompt: Option<String>,
    supports_vision: Option<bool>,
    supports_tools: Option<bool>,
    supports_reasoning: Option<bool>,
    supports_streaming: Option<bool>,
    price_input_per_m: Option<f64>,
    price_output_per_m: Option<f64>,
    #[serde(default)]
    forced_skills: Vec<String>,
}

fn private() -> String {
    "private".into()
}

fn check_model(req: &ModelReq) -> AppResult<()> {
    if req.upstream_id.trim().is_empty() {
        return Err(AppError::bad_request("Model ID is required"));
    }
    if !matches!(req.visibility.as_str(), "public" | "private") {
        return Err(AppError::bad_request("Visibility must be public or private"));
    }
    if req.default_temperature.is_some_and(|t| !(0.0..=2.0).contains(&t)) {
        return Err(AppError::bad_request("Temperature must be between 0 and 2"));
    }
    if req.name.as_deref().is_some_and(|n| n.trim().is_empty() || n.contains(char::is_whitespace)) {
        return Err(AppError::bad_request("API name cannot be empty or contain spaces"));
    }
    Ok(())
}

async fn insert_model(state: &AppState, req: &ModelReq) -> AppResult<Uuid> {
    let slug: String = sqlx::query_scalar("select slug from providers where id = $1")
        .bind(req.provider_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Provider"))?;
    let upstream_id = req.upstream_id.trim();
    let known = upstream::known_model(upstream_id);
    let name = req.name.clone().map(|n| n.trim().to_string()).unwrap_or_else(|| format!("{slug}/{upstream_id}"));
    let display = req.display_name.clone().filter(|d| !d.trim().is_empty()).unwrap_or_else(|| upstream_id.to_string());
    Ok(sqlx::query_scalar(
        "insert into models (provider_id, upstream_id, name, display_name, description, visibility, enabled, context_window,
            max_output_tokens, default_temperature, system_prompt, supports_vision, supports_tools, supports_reasoning,
            supports_streaming, price_input_per_m, price_output_per_m, forced_skills)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18) returning id",
    )
    .bind(req.provider_id)
    .bind(upstream_id)
    .bind(&name)
    .bind(display.trim())
    .bind(&req.description)
    .bind(&req.visibility)
    .bind(req.enabled)
    .bind(req.context_window.or(known.context_window))
    .bind(req.max_output_tokens)
    .bind(req.default_temperature)
    .bind(req.system_prompt.as_deref().filter(|s| !s.trim().is_empty()))
    .bind(req.supports_vision.unwrap_or(known.vision))
    .bind(req.supports_tools.unwrap_or(known.tools))
    .bind(req.supports_reasoning.unwrap_or(known.reasoning))
    .bind(req.supports_streaming.unwrap_or(true))
    .bind(req.price_input_per_m)
    .bind(req.price_output_per_m)
    .bind(&req.forced_skills)
    .fetch_one(&state.db)
    .await
    .map_err(|e| match AppError::from(e) {
        e if e.status == axum::http::StatusCode::CONFLICT => {
            AppError::conflict(format!("A model named {name} or with this ID on this provider already exists"))
        }
        e => e,
    })?)
}

async fn create_model(State(state): State<AppState>, admin: Admin, Json(req): Json<ModelReq>) -> AppResult<Json<Value>> {
    check_model(&req)?;
    let id = insert_model(&state, &req).await?;
    audit::record(
        &state,
        &admin.0,
        "model.created",
        "model",
        id,
        json!({ "provider_id": req.provider_id, "upstream_id": req.upstream_id, "visibility": req.visibility }),
    )
    .await;
    Ok(Json(json!({ "id": id })))
}

#[derive(Deserialize)]
struct ImportReq {
    provider_id: Uuid,
    upstream_ids: Vec<String>,
    #[serde(default = "private")]
    visibility: String,
}

async fn import_models(State(state): State<AppState>, admin: Admin, Json(req): Json<ImportReq>) -> AppResult<Json<Value>> {
    let mut created = vec![];
    let mut errors = vec![];
    for upstream_id in &req.upstream_ids {
        let m = ModelReq {
            provider_id: req.provider_id,
            upstream_id: upstream_id.clone(),
            name: None,
            display_name: None,
            description: String::new(),
            visibility: req.visibility.clone(),
            enabled: true,
            context_window: None,
            max_output_tokens: None,
            default_temperature: None,
            system_prompt: None,
            supports_vision: None,
            supports_tools: None,
            supports_reasoning: None,
            supports_streaming: None,
            price_input_per_m: None,
            price_output_per_m: None,
            forced_skills: vec![],
        };
        match check_model(&m) {
            Ok(()) => match insert_model(&state, &m).await {
                Ok(id) => created.push(id),
                Err(e) => errors.push(json!({ "upstream_id": upstream_id, "error": e.message })),
            },
            Err(e) => errors.push(json!({ "upstream_id": upstream_id, "error": e.message })),
        }
    }
    audit::record(
        &state,
        &admin.0,
        "model.imported",
        "provider",
        req.provider_id,
        json!({ "upstream_ids": req.upstream_ids, "visibility": req.visibility, "created": created.len() }),
    )
    .await;
    Ok(Json(json!({ "created": created, "errors": errors })))
}

async fn update_model(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(req): Json<ModelReq>,
) -> AppResult<Json<Value>> {
    check_model(&req)?;
    let name = req.name.as_deref().map(str::trim).ok_or_else(|| AppError::bad_request("API name is required"))?;
    let n = sqlx::query(
        "update models set upstream_id = $2, name = $3, display_name = $4, description = $5, visibility = $6, enabled = $7,
            context_window = $8, max_output_tokens = $9, default_temperature = $10, system_prompt = $11, supports_vision = $12,
            supports_tools = $13, supports_reasoning = $14, supports_streaming = $15, price_input_per_m = $16,
            price_output_per_m = $17, forced_skills = $18
         where id = $1",
    )
    .bind(id)
    .bind(req.upstream_id.trim())
    .bind(name)
    .bind(req.display_name.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(req.upstream_id.trim()))
    .bind(&req.description)
    .bind(&req.visibility)
    .bind(req.enabled)
    .bind(req.context_window)
    .bind(req.max_output_tokens)
    .bind(req.default_temperature)
    .bind(req.system_prompt.as_deref().filter(|s| !s.trim().is_empty()))
    .bind(req.supports_vision.unwrap_or(false))
    .bind(req.supports_tools.unwrap_or(false))
    .bind(req.supports_reasoning.unwrap_or(false))
    .bind(req.supports_streaming.unwrap_or(true))
    .bind(req.price_input_per_m)
    .bind(req.price_output_per_m)
    .bind(&req.forced_skills)
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("Model"));
    }
    audit::record(
        &state,
        &admin.0,
        "model.updated",
        "model",
        id,
        json!({ "name": name, "visibility": req.visibility, "enabled": req.enabled, "upstream_id": req.upstream_id }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_model(State(state): State<AppState>, admin: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let name: String = sqlx::query_scalar("delete from models where id = $1 returning name")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Model"))?;
    audit::record(&state, &admin.0, "model.deleted", "model", id, json!({ "name": name })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn model_grants(State(state): State<AppState>, _: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let rows: Vec<(Option<Uuid>, Option<String>, Option<String>, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "select u.id, u.name, u.email, g.id, g.name from grants gr
         left join users u on u.id = gr.user_id left join groups g on g.id = gr.group_id
         where gr.model_id = $1",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let users: Vec<Value> =
        rows.iter().filter_map(|r| r.0.map(|id| json!({ "id": id, "name": r.1, "email": r.2 }))).collect();
    let groups: Vec<Value> = rows.iter().filter_map(|r| r.3.map(|id| json!({ "id": id, "name": r.4 }))).collect();
    Ok(Json(json!({ "users": users, "groups": groups })))
}

#[derive(Deserialize)]
struct GrantSet {
    user_ids: Vec<Uuid>,
    group_ids: Vec<Uuid>,
}

async fn set_model_grants(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(req): Json<GrantSet>,
) -> AppResult<Json<Value>> {
    let mut tx = state.db.begin().await?;
    sqlx::query("delete from grants where model_id = $1").bind(id).execute(&mut *tx).await?;
    sqlx::query("insert into grants (model_id, user_id) select $1, unnest($2::uuid[])")
    .bind(id)
    .bind(&req.user_ids)
    .execute(&mut *tx)
    .await?;
    sqlx::query("insert into grants (model_id, group_id) select $1, unnest($2::uuid[])")
        .bind(id)
        .bind(&req.group_ids)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    audit::record(
        &state,
        &admin.0,
        "model.grants_set",
        "model",
        id,
        json!({ "user_ids": req.user_ids, "group_ids": req.group_ids }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ImpactQ {
    visibility: String,
}

/// How many active users would lose access if the model switched to this visibility.
async fn visibility_impact(
    State(state): State<AppState>,
    _: Admin,
    Path(id): Path<Uuid>,
    Query(q): Query<ImpactQ>,
) -> AppResult<Json<Value>> {
    if q.visibility != "private" {
        return Ok(Json(json!({ "users_losing_access": 0 })));
    }
    let n: i64 = sqlx::query_scalar(
        "select count(*) from users u where u.status = 'active' and not exists (
            select 1 from grants g where g.model_id = $1 and (g.user_id = u.id or g.group_id in (
                select gm.group_id from group_members gm where gm.user_id = u.id
                union select id from groups where is_everyone)))",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(json!({ "users_losing_access": n })))
}
