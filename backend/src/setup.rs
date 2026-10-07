use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::{ConnectInfo, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::post,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth, crypto,
    error::{AppError, AppResult},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/setup", post(create_admin))
}

#[derive(Deserialize)]
struct SetupReq {
    token: String,
    name: String,
    email: String,
    password: String,
}

async fn create_admin(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<SetupReq>,
) -> AppResult<impl IntoResponse> {
    let expected = state.setup_token.lock().unwrap().clone().ok_or_else(|| AppError::forbidden("Setup is already complete"))?;
    if !constant_time_eq(req.token.trim().as_bytes(), expected.as_bytes()) {
        return Err(AppError::forbidden("Wrong setup token. Check the server logs for the current one."));
    }
    let email = auth::normalize_email(&req.email)?;
    let name = auth::require_name(&req.name)?;
    auth::validate_password(&state, &req.password).await?;
    // The advisory lock serializes concurrent setup attempts so exactly one admin wins.
    let mut tx = state.db.begin().await?;
    sqlx::query("select pg_advisory_xact_lock(7362001)").execute(&mut *tx).await?;
    let exists: bool = sqlx::query_scalar("select exists(select 1 from users)").fetch_one(&mut *tx).await?;
    if exists {
        return Err(AppError::forbidden("Setup is already complete"));
    }
    let id: Uuid = sqlx::query_scalar(
        "insert into users (email, name, password_hash, role) values ($1, $2, $3, 'admin') returning id",
    )
    .bind(&email)
    .bind(name)
    .bind(crypto::hash_password(&req.password)?)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "insert into audit_log (actor_id, actor_email, action, target_type, target_id, ip)
         values ($1, $2, 'setup.admin_created', 'user', $1::text, $3)",
    )
    .bind(id)
    .bind(&email)
    .bind(state.client_ip(&headers, Some(addr)))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    *state.setup_token.lock().unwrap() = None;
    let cookie = auth::start_session(&state, id, state.client_ip(&headers, Some(addr)), &headers).await?;
    Ok((cookie, Json(json!({ "ok": true }))))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
