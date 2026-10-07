use std::{net::SocketAddr, time::Instant};

use axum::{
    Json, Router,
    extract::{ConnectInfo, FromRequestParts, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    response::{AppendHeaders, IntoResponse},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use uuid::Uuid;

use crate::{
    crypto,
    error::{AppError, AppResult},
    settings,
    state::AppState,
};

const SESSION_COOKIE: &str = "braid_session";
const SESSION_DAYS: i64 = 30;
const MAX_LOGIN_FAILURES: u32 = 5;
const LOCKOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct CurrentUser {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: String,
    pub must_reset_password: bool,
    pub totp_enabled: bool,
    #[sqlx(skip)]
    pub must_enroll_2fa: bool,
    #[sqlx(skip)]
    #[serde(skip)]
    pub ip: Option<String>,
}

impl CurrentUser {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

pub fn requires_2fa(policy: &str, role: &str) -> bool {
    policy == "everyone" || (policy == "admins" && role == "admin")
}

pub struct Admin(pub CurrentUser);

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

pub fn peer(parts: &Parts) -> Option<SocketAddr> {
    parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0)
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> AppResult<Self> {
        let token = cookie_value(&parts.headers, SESSION_COOKIE).ok_or_else(AppError::unauthorized)?;
        let mut user: CurrentUser = sqlx::query_as(
            "with s as (
                select u.id, u.email, u.name, u.role, u.must_reset_password, u.totp_enabled, u.last_active_at
                from sessions s join users u on u.id = s.user_id
                where s.token_hash = $1 and s.expires_at > now() and u.status = 'active'
             ), touch as (
                update users set last_active_at = now()
                where id = (select id from s) and coalesce((select last_active_at from s) < now() - interval '5 minutes', true)
             )
             select id, email, name, role, must_reset_password, totp_enabled from s",
        )
        .bind(crypto::sha256(token))
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unauthorized)?;
        user.ip = state.client_ip(&parts.headers, peer(parts));
        if !user.totp_enabled {
            let policy = settings::get(state).await?.require_2fa;
            user.must_enroll_2fa = requires_2fa(&policy, &user.role);
            // Until enrolled, only what is needed to enroll (or leave) is reachable.
            let path = parts.uri.path();
            if user.must_enroll_2fa && !(path == "/api/auth/me" || path == "/api/auth/logout" || path.starts_with("/api/me/2fa")) {
                return Err(AppError::new(StatusCode::FORBIDDEN, "2fa_required", "Set up two-factor authentication in Settings first"));
            }
        }
        Ok(user)
    }
}

impl FromRequestParts<AppState> for Admin {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> AppResult<Self> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        if !user.is_admin() {
            return Err(AppError::forbidden("Admins only"));
        }
        Ok(Admin(user))
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
        .route("/api/auth/password", post(change_password))
        .route("/api/auth/signup", post(signup))
        .route("/api/auth/invite/{token}", get(get_invite).post(accept_invite))
        .route("/api/auth/reset/{token}", post(reset_password))
        .route("/api/auth/forgot", post(forgot_password))
        .route("/api/me/2fa/setup", post(totp_setup))
        .route("/api/me/2fa/enable", post(totp_enable))
        .route("/api/me/2fa/disable", post(totp_disable))
        .route("/api/instance", get(instance))
}

pub async fn start_session(
    state: &AppState,
    user_id: Uuid,
    ip: Option<String>,
    headers: &HeaderMap,
) -> AppResult<AppendHeaders<[(header::HeaderName, HeaderValue); 1]>> {
    let token = crypto::random_token(32);
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(|s| s.chars().take(300).collect::<String>());
    sqlx::query("insert into sessions (token_hash, user_id, expires_at, ip, user_agent) values ($1, $2, $3, $4, $5)")
        .bind(crypto::sha256(&token))
        .bind(user_id)
        .bind(Utc::now() + Duration::days(SESSION_DAYS))
        .bind(ip)
        .bind(ua)
        .execute(&state.db)
        .await?;
    Ok(set_cookie(state, &token, SESSION_DAYS * 86400))
}

fn set_cookie(state: &AppState, value: &str, max_age: i64) -> AppendHeaders<[(header::HeaderName, HeaderValue); 1]> {
    let secure = if state.config.secure_cookies { "; Secure" } else { "" };
    let cookie = format!("{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}");
    AppendHeaders([(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap())])
}

/// Min 12 chars and not in the Have I Been Pwned corpus (k-anonymity range API).
/// Fails open when HIBP is unreachable so air-gapped installs still work.
pub async fn validate_password(state: &AppState, password: &str) -> AppResult<()> {
    if password.chars().count() < 12 {
        return Err(AppError::bad_request("Password must be at least 12 characters"));
    }
    let digest = hex::encode_upper(Sha1::digest(password.as_bytes()));
    let (prefix, suffix) = digest.split_at(5);
    let res = state
        .http
        .get(format!("https://api.pwnedpasswords.com/range/{prefix}"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await;
    match res {
        Ok(r) if r.status().is_success() => {
            let body = r.text().await.unwrap_or_default();
            if body.lines().any(|l| l.split(':').next() == Some(suffix)) {
                return Err(AppError::bad_request(
                    "This password appears in a known data breach; choose a different one",
                ));
            }
        }
        _ => tracing::warn!("breached-password check unavailable; skipping"),
    }
    Ok(())
}

#[derive(Deserialize)]
struct LoginReq {
    email: String,
    password: String,
    code: Option<String>,
}

async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> AppResult<impl IntoResponse> {
    let key = req.email.trim().to_lowercase();
    {
        let failures = state.login_failures.lock().unwrap();
        if let Some((n, at)) = failures.get(&key) {
            if *n >= MAX_LOGIN_FAILURES && at.elapsed() < LOCKOUT {
                return Err(AppError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "locked",
                    "Too many failed attempts. Try again in 15 minutes.",
                ));
            }
        }
    }
    let row: Option<(Uuid, Option<String>, String, bool)> =
        sqlx::query_as("select id, password_hash, status, totp_enabled from users where lower(email) = $1")
            .bind(&key)
            .fetch_optional(&state.db)
            .await?;
    let ok = match &row {
        Some((_, Some(hash), status, _)) => crypto::verify_password(&req.password, hash) && status == "active",
        _ => {
            // Equalize timing with the found-user path.
            static DUMMY: std::sync::LazyLock<String> =
                std::sync::LazyLock::new(|| crypto::hash_password("braid-dummy-password").unwrap());
            crypto::verify_password(&req.password, &DUMMY);
            false
        }
    };
    if !ok {
        // ponytail: in-memory per-email counter; resets on restart and is per-node. Move to the DB or Valkey for multi-node.
        let mut failures = state.login_failures.lock().unwrap();
        let e = failures.entry(key).or_insert((0, Instant::now()));
        if e.1.elapsed() >= LOCKOUT {
            *e = (0, Instant::now());
        }
        e.0 += 1;
        e.1 = Instant::now();
        return Err(AppError::new(StatusCode::UNAUTHORIZED, "invalid_credentials", "Wrong email or password"));
    }
    let (user_id, _, _, totp) = row.unwrap();
    if totp {
        let Some(code) = req.code.as_deref().filter(|c| !c.trim().is_empty()) else {
            return Err(AppError::new(StatusCode::UNAUTHORIZED, "totp_required", "Enter the 6-digit code from your authenticator app"));
        };
        if !check_totp(&state, user_id, code).await? {
            let mut failures = state.login_failures.lock().unwrap();
            let e = failures.entry(key).or_insert((0, Instant::now()));
            e.0 += 1;
            e.1 = Instant::now();
            return Err(AppError::new(StatusCode::UNAUTHORIZED, "totp_invalid", "That code is wrong or was already used"));
        }
    }
    state.login_failures.lock().unwrap().remove(&key);
    let ip = state.client_ip(&headers, Some(addr));
    let cookie = start_session(&state, user_id, ip, &headers).await?;
    Ok((cookie, Json(json!({ "ok": true }))))
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> AppResult<impl IntoResponse> {
    if let Some(token) = cookie_value(&headers, SESSION_COOKIE) {
        sqlx::query("delete from sessions where token_hash = $1").bind(crypto::sha256(token)).execute(&state.db).await?;
    }
    Ok((set_cookie(&state, "", 0), Json(json!({ "ok": true }))))
}

async fn me(State(state): State<AppState>, user: CurrentUser) -> AppResult<Json<Value>> {
    let s = settings::get(&state).await?;
    let default_model: Option<Uuid> =
        sqlx::query_scalar("select default_model_id from users where id = $1").bind(user.id).fetch_one(&state.db).await?;
    let can_use_api_keys = crate::access::can_use_api_keys(&state, user.id).await?;
    Ok(Json(json!({
        "user": user,
        "default_model_id": default_model.or(s.default_model_id),
        "can_use_api_keys": can_use_api_keys,
        "show_metrics": s.show_metrics,
    })))
}

async fn instance(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let s = settings::get(&state).await?;
    let needs_setup = state.setup_token.lock().unwrap().is_some();
    Ok(Json(json!({
        "name": s.name,
        "logo_url": s.logo_url,
        "needs_setup": needs_setup,
        "open_signup": s.signup_mode == "open",
        "email_enabled": crate::mail::enabled(&state).await,
    })))
}

#[derive(Deserialize)]
struct ChangePasswordReq {
    current: String,
    new: String,
}

async fn change_password(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<ChangePasswordReq>,
) -> AppResult<Json<Value>> {
    let hash: Option<String> =
        sqlx::query_scalar("select password_hash from users where id = $1").bind(user.id).fetch_one(&state.db).await?;
    if !hash.is_some_and(|h| crypto::verify_password(&req.current, &h)) {
        return Err(AppError::bad_request("Current password is wrong"));
    }
    validate_password(&state, &req.new).await?;
    sqlx::query("update users set password_hash = $2, must_reset_password = false where id = $1")
        .bind(user.id)
        .bind(crypto::hash_password(&req.new)?)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct SignupReq {
    name: String,
    email: String,
    password: String,
}

async fn signup(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<SignupReq>,
) -> AppResult<impl IntoResponse> {
    let s = settings::get(&state).await?;
    if s.signup_mode != "open" {
        return Err(AppError::forbidden("Sign-up is by invitation only"));
    }
    let email = normalize_email(&req.email)?;
    let domain = email.rsplit('@').next().unwrap_or_default();
    if !s.signup_domains.is_empty() && !s.signup_domains.iter().any(|d| d.eq_ignore_ascii_case(domain)) {
        return Err(AppError::forbidden("Sign-up is not open for this email domain"));
    }
    let name = require_name(&req.name)?;
    validate_password(&state, &req.password).await?;
    let id: Uuid = sqlx::query_scalar(
        "insert into users (email, name, password_hash, role) values ($1, $2, $3, 'user') returning id",
    )
    .bind(&email)
    .bind(name)
    .bind(crypto::hash_password(&req.password)?)
    .fetch_one(&state.db)
    .await
    .map_err(|e| match AppError::from(e) {
        e if e.status == StatusCode::CONFLICT => AppError::conflict("An account with this email already exists"),
        e => e,
    })?;
    let cookie = start_session(&state, id, state.client_ip(&headers, Some(addr)), &headers).await?;
    Ok((cookie, Json(json!({ "ok": true }))))
}

pub fn normalize_email(email: &str) -> AppResult<String> {
    let email = email.trim().to_lowercase();
    let valid = email.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.'));
    if !valid || email.len() > 254 || email.contains(char::is_whitespace) {
        return Err(AppError::bad_request(format!("Invalid email: {email}")));
    }
    Ok(email)
}

pub fn require_name(name: &str) -> AppResult<&str> {
    let name = name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::bad_request("Name is required (max 100 characters)"));
    }
    Ok(name)
}

async fn get_invite(State(state): State<AppState>, Path(token): Path<String>) -> AppResult<Json<Value>> {
    let email: String = sqlx::query_scalar(
        "select email from invitations
         where token_hash = $1 and accepted_at is null and revoked_at is null and expires_at > now()",
    )
    .bind(crypto::sha256(&token))
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("Invitation (it may have expired or been used)"))?;
    Ok(Json(json!({ "email": email })))
}

#[derive(Deserialize)]
struct AcceptInviteReq {
    name: String,
    password: String,
}

async fn accept_invite(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(req): Json<AcceptInviteReq>,
) -> AppResult<impl IntoResponse> {
    let name = require_name(&req.name)?;
    validate_password(&state, &req.password).await?;
    let mut tx = state.db.begin().await?;
    let invite: (Uuid, String, String, Vec<Uuid>) = sqlx::query_as(
        "update invitations set accepted_at = now()
         where token_hash = $1 and accepted_at is null and revoked_at is null and expires_at > now()
         returning id, email, role, group_ids",
    )
    .bind(crypto::sha256(&token))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("Invitation (it may have expired or been used)"))?;
    let user_id: Uuid = sqlx::query_scalar(
        "insert into users (email, name, password_hash, role) values ($1, $2, $3, $4) returning id",
    )
    .bind(&invite.1)
    .bind(name)
    .bind(crypto::hash_password(&req.password)?)
    .bind(&invite.2)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "insert into group_members (group_id, user_id)
         select g.id, $2 from groups g where g.id = any($1) and not g.is_everyone on conflict do nothing",
    )
    .bind(&invite.3)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let cookie = start_session(&state, user_id, state.client_ip(&headers, Some(addr)), &headers).await?;
    Ok((cookie, Json(json!({ "ok": true }))))
}

#[derive(Deserialize)]
struct ResetReq {
    password: String,
}

async fn reset_password(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(req): Json<ResetReq>,
) -> AppResult<Json<Value>> {
    validate_password(&state, &req.password).await?;
    let mut tx = state.db.begin().await?;
    let user_id: Uuid = sqlx::query_scalar(
        "update password_resets set used_at = now()
         where token_hash = $1 and used_at is null and expires_at > now() returning user_id",
    )
    .bind(crypto::sha256(&token))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("Reset link (it may have expired or been used)"))?;
    sqlx::query("update users set password_hash = $2, must_reset_password = false where id = $1")
        .bind(user_id)
        .bind(crypto::hash_password(&req.password)?)
        .execute(&mut *tx)
        .await?;
    sqlx::query("delete from sessions where user_id = $1").bind(user_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ForgotReq {
    email: String,
}

/// Always answers the same way so it cannot be used to discover which emails have accounts.
async fn forgot_password(State(state): State<AppState>, Json(req): Json<ForgotReq>) -> AppResult<Json<Value>> {
    let email = req.email.trim().to_lowercase();
    let user: Option<Uuid> = sqlx::query_scalar("select id from users where lower(email) = $1 and status = 'active'")
        .bind(&email)
        .fetch_optional(&state.db)
        .await?;
    if let Some(id) = user {
        let recent: bool = sqlx::query_scalar(
            "select exists(select 1 from password_resets where user_id = $1 and expires_at > now() + interval '23 hours 55 minutes')",
        )
        .bind(id)
        .fetch_one(&state.db)
        .await?;
        if !recent && crate::mail::enabled(&state).await {
            let token = crypto::random_token(32);
            sqlx::query("insert into password_resets (token_hash, user_id, expires_at) values ($1, $2, $3)")
                .bind(crypto::sha256(&token))
                .bind(id)
                .bind(Utc::now() + Duration::hours(24))
                .execute(&state.db)
                .await?;
            let link = format!("{}/reset/{token}", state.config.public_url);
            crate::mail::send_later(
                &state,
                email,
                "Reset your password".into(),
                format!("Someone asked to reset the password for this account. If it was you, open:\n{link}\n\nThe link expires in 24 hours. Otherwise ignore this email."),
            );
        }
    }
    Ok(Json(json!({ "ok": true })))
}

/// Verifies a TOTP code and records its step so the same code cannot be used twice.
async fn check_totp(state: &AppState, user_id: Uuid, code: &str) -> AppResult<bool> {
    let row: Option<(Option<Vec<u8>>, Option<i64>)> =
        sqlx::query_as("select totp_secret, totp_last_step from users where id = $1").bind(user_id).fetch_optional(&state.db).await?;
    let Some((Some(ct), last)) = row else { return Ok(false) };
    let secret = hex::decode(state.secrets.decrypt(&ct)?).map_err(|e| anyhow::anyhow!(e))?;
    let now = Utc::now().timestamp() as u64;
    let Some(step) = crypto::totp_match(&secret, code, now) else { return Ok(false) };
    let updated = sqlx::query(
        "update users set totp_last_step = $2 where id = $1 and (totp_last_step is null or totp_last_step < $2)",
    )
    .bind(user_id)
    .bind(step as i64)
    .execute(&state.db)
    .await?
    .rows_affected();
    Ok(updated == 1 && last.is_none_or(|l| (step as i64) > l))
}

async fn totp_setup(State(state): State<AppState>, user: CurrentUser) -> AppResult<Json<Value>> {
    if user.totp_enabled {
        return Err(AppError::conflict("Two-factor authentication is already on. Turn it off first."));
    }
    let mut raw = [0u8; 20];
    rand::fill(&mut raw);
    let secret = crypto::base32(&raw);
    sqlx::query("update users set totp_secret = $2, totp_last_step = null where id = $1")
        .bind(user.id)
        .bind(state.secrets.encrypt(&hex::encode(raw)))
        .execute(&state.db)
        .await?;
    let issuer = settings::get(&state).await?.name;
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    Ok(Json(json!({
        "secret": secret,
        "otpauth_url": format!("otpauth://totp/{}:{}?secret={secret}&issuer={}&algorithm=SHA1&digits=6&period=30", enc(&issuer), enc(&user.email), enc(&issuer)),
    })))
}

#[derive(Deserialize)]
struct TotpCode {
    code: String,
}

async fn totp_enable(State(state): State<AppState>, user: CurrentUser, Json(req): Json<TotpCode>) -> AppResult<Json<Value>> {
    if user.totp_enabled {
        return Err(AppError::conflict("Two-factor authentication is already on"));
    }
    if !check_totp(&state, user.id, &req.code).await? {
        return Err(AppError::bad_request("That code doesn't match. Check your device's clock and try the next code."));
    }
    sqlx::query("update users set totp_enabled = true where id = $1").bind(user.id).execute(&state.db).await?;
    crate::audit::record(&state, &user, "user.2fa_enabled", "user", user.id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct TotpDisable {
    password: String,
}

async fn totp_disable(State(state): State<AppState>, user: CurrentUser, Json(req): Json<TotpDisable>) -> AppResult<Json<Value>> {
    if requires_2fa(&settings::get(&state).await?.require_2fa, &user.role) {
        return Err(AppError::forbidden("Your organization requires two-factor authentication"));
    }
    let hash: Option<String> =
        sqlx::query_scalar("select password_hash from users where id = $1").bind(user.id).fetch_one(&state.db).await?;
    if !hash.is_some_and(|h| crypto::verify_password(&req.password, &h)) {
        return Err(AppError::bad_request("Password is wrong"));
    }
    sqlx::query("update users set totp_enabled = false, totp_secret = null, totp_last_step = null where id = $1")
        .bind(user.id)
        .execute(&state.db)
        .await?;
    crate::audit::record(&state, &user, "user.2fa_disabled", "user", user.id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}
