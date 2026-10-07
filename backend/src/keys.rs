//! Platform API keys (users manage their own; admins see and revoke all) and the signed-in user's profile.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, patch},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    access, audit,
    auth::{self, Admin, CurrentUser},
    crypto,
    error::{AppError, AppResult},
    gateway, settings,
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/me", patch(update_profile))
        .route("/api/me/usage", get(my_usage))
        .route("/api/me/api-keys", get(my_keys).post(create_key))
        .route("/api/me/api-keys/{id}", delete(revoke_own))
        .route("/api/admin/api-keys", get(all_keys))
        .route("/api/admin/api-keys/{id}", delete(revoke_any))
}

#[derive(Deserialize)]
struct ProfileReq {
    name: String,
}

async fn update_profile(State(state): State<AppState>, user: CurrentUser, Json(req): Json<ProfileReq>) -> AppResult<Json<Value>> {
    let name = auth::require_name(&req.name)?;
    sqlx::query("update users set name = $2 where id = $1").bind(user.id).bind(name).execute(&state.db).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct DaysQ {
    days: Option<i32>,
}

async fn my_usage(State(state): State<AppState>, user: CurrentUser, Query(q): Query<DaysQ>) -> AppResult<Json<Value>> {
    Ok(Json(gateway::user_usage(&state, user.id, q.days.unwrap_or(30)).await?))
}

#[derive(Serialize, sqlx::FromRow)]
struct KeyRow {
    id: Uuid,
    name: String,
    prefix: String,
    model_ids: Option<Vec<Uuid>>,
    allow_skills: bool,
    rate_limit_rpm: Option<i32>,
    token_quota_month: Option<i64>,
    expires_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    last_used_ip: Option<String>,
    created_at: DateTime<Utc>,
}

const KEY_COLS: &str = "id, name, prefix, model_ids, allow_skills, rate_limit_rpm, token_quota_month, expires_at, revoked_at,
    last_used_at, last_used_ip, created_at";

async fn my_keys(State(state): State<AppState>, user: CurrentUser) -> AppResult<Json<Value>> {
    let keys: Vec<KeyRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("select {KEY_COLS} from api_keys where user_id = $1 order by created_at desc")))
            .bind(user.id)
            .fetch_all(&state.db)
            .await?;
    let s = settings::get(&state).await?;
    Ok(Json(json!({
        "keys": keys,
        "allowed": access::can_use_api_keys(&state, user.id).await?,
        "max_keys": s.max_api_keys_per_user,
        "max_rpm": s.max_api_key_rpm,
    })))
}

#[derive(Deserialize)]
struct NewKey {
    name: String,
    expires_in_days: Option<i64>,
    model_ids: Option<Vec<Uuid>>,
    #[serde(default = "yes")]
    allow_skills: bool,
    rate_limit_rpm: Option<i32>,
    token_quota_month: Option<i64>,
}

fn yes() -> bool {
    true
}

async fn create_key(State(state): State<AppState>, user: CurrentUser, Json(req): Json<NewKey>) -> AppResult<Json<Value>> {
    if !access::can_use_api_keys(&state, user.id).await? {
        return Err(AppError::forbidden("API keys are not enabled for your account. Ask an admin."));
    }
    let s = settings::get(&state).await?;
    let active: i64 = sqlx::query_scalar(
        "select count(*) from api_keys where user_id = $1 and revoked_at is null and (expires_at is null or expires_at > now())",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    if active >= s.max_api_keys_per_user {
        return Err(AppError::conflict(format!("You already have the maximum of {} active keys", s.max_api_keys_per_user)));
    }
    let name = auth::require_name(&req.name)?;
    if req.rate_limit_rpm.is_some_and(|r| r < 1) || req.token_quota_month.is_some_and(|q| q < 1) {
        return Err(AppError::bad_request("Limits must be positive"));
    }
    if let (Some(r), Some(max)) = (req.rate_limit_rpm, s.max_api_key_rpm) {
        if r > max {
            return Err(AppError::bad_request(format!("Rate limit cannot exceed the admin maximum of {max}/min")));
        }
    }
    let model_ids = req.model_ids.filter(|ids| !ids.is_empty());
    if let Some(ids) = &model_ids {
        let mine = access::accessible_models(&state, user.id).await?;
        if ids.iter().any(|id| !mine.iter().any(|m| m.id == *id)) {
            return Err(AppError::bad_request("A key can only be limited to models you can use"));
        }
    }
    let key = format!("sk-braid-{}", crypto::random_token(30));
    let prefix: String = key.chars().take(13).collect();
    let expires = req.expires_in_days.filter(|d| *d > 0).map(|d| Utc::now() + Duration::days(d.min(3650)));
    let row: KeyRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "insert into api_keys (user_id, name, prefix, key_hash, model_ids, allow_skills, rate_limit_rpm, token_quota_month, expires_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9) returning {KEY_COLS}"
    )))
    .bind(user.id)
    .bind(name)
    .bind(&prefix)
    .bind(state.secrets.api_key_hash(&key))
    .bind(&model_ids)
    .bind(req.allow_skills)
    .bind(req.rate_limit_rpm)
    .bind(req.token_quota_month)
    .bind(expires)
    .fetch_one(&state.db)
    .await?;
    audit::record(&state, &user, "api_key.created", "api_key", row.id, json!({ "name": name, "prefix": prefix })).await;
    Ok(Json(json!({ "key": key, "api_key": row })))
}

async fn revoke(state: &AppState, actor: &CurrentUser, id: Uuid, owner: Option<Uuid>) -> AppResult<Json<Value>> {
    let row: (String, String) = sqlx::query_as(
        "update api_keys set revoked_at = now() where id = $1 and revoked_at is null and ($2::uuid is null or user_id = $2)
         returning name, prefix",
    )
    .bind(id)
    .bind(owner)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("Active key"))?;
    audit::record(state, actor, "api_key.revoked", "api_key", id, json!({ "name": row.0, "prefix": row.1 })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn revoke_own(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    revoke(&state, &user, id, Some(user.id)).await
}

async fn revoke_any(State(state): State<AppState>, admin: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    revoke(&state, &admin.0, id, None).await
}

async fn all_keys(State(state): State<AppState>, _: Admin) -> AppResult<Json<Value>> {
    let rows: Vec<(Uuid, String, String, String, String, Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>)> =
        sqlx::query_as(
            "select k.id, k.name, k.prefix, u.name, u.email, k.expires_at, k.last_used_at, k.last_used_ip, k.created_at, k.revoked_at
             from api_keys k join users u on u.id = k.user_id order by k.revoked_at nulls first, k.created_at desc limit 500",
        )
        .fetch_all(&state.db)
        .await?;
    Ok(Json(json!(rows
        .iter()
        .map(|r| json!({
            "id": r.0, "name": r.1, "prefix": r.2, "user_name": r.3, "user_email": r.4, "expires_at": r.5,
            "last_used_at": r.6, "last_used_ip": r.7, "created_at": r.8, "revoked_at": r.9,
        }))
        .collect::<Vec<_>>())))
}
