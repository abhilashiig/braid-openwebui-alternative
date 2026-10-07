use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppResult;
use crate::state::AppState;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InstanceSettings {
    pub name: String,
    pub logo_url: Option<String>,
    pub default_model_id: Option<Uuid>,
    /// "invite" or "open"
    pub signup_mode: String,
    pub signup_domains: Vec<String>,
    pub allow_api_keys: bool,
    pub max_api_keys_per_user: i64,
    pub max_api_key_rpm: Option<i32>,
    pub invite_ttl_days: i64,
    /// Hosts or IPs allowed despite the private-range block (e.g. a local Ollama).
    pub allowed_private_hosts: Vec<String>,
    pub show_metrics: bool,
    pub max_tool_calls: i64,
    pub log_content: bool,
    pub smtp_host: Option<String>,
    pub smtp_port: u16,
    pub smtp_username: Option<String>,
    pub smtp_from: Option<String>,
    /// starttls | tls | none
    pub smtp_tls: String,
}

impl Default for InstanceSettings {
    fn default() -> Self {
        Self {
            name: "Braid".into(),
            logo_url: None,
            default_model_id: None,
            signup_mode: "invite".into(),
            signup_domains: vec![],
            allow_api_keys: false,
            max_api_keys_per_user: 5,
            max_api_key_rpm: None,
            invite_ttl_days: 7,
            allowed_private_hosts: vec![],
            show_metrics: true,
            max_tool_calls: 5,
            log_content: false,
            smtp_host: None,
            smtp_port: 587,
            smtp_username: None,
            smtp_from: None,
            smtp_tls: "starttls".into(),
        }
    }
}

pub async fn get(state: &AppState) -> AppResult<InstanceSettings> {
    let v: Option<serde_json::Value> =
        sqlx::query_scalar("select value from settings where key = 'instance'").fetch_optional(&state.db).await?;
    let mut s: InstanceSettings = v.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    apply_env(&mut s);
    Ok(s)
}

/// Environment variables for headless deploys override whatever was saved in the UI (FR-SET-08).
pub fn apply_env(s: &mut InstanceSettings) -> Vec<&'static str> {
    let mut applied = vec![];
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let list = |v: String| v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect::<Vec<_>>();
    let truthy = |v: &str| v == "true" || v == "1";
    if let Some(v) = var("BRAID_INSTANCE_NAME") { s.name = v; applied.push("name"); }
    if let Some(v) = var("BRAID_SIGNUP_MODE") { s.signup_mode = v; applied.push("signup_mode"); }
    if let Some(v) = var("BRAID_SIGNUP_DOMAINS") { s.signup_domains = list(v); applied.push("signup_domains"); }
    if let Some(v) = var("BRAID_ALLOW_API_KEYS") { s.allow_api_keys = truthy(&v); applied.push("allow_api_keys"); }
    if let Some(v) = var("BRAID_ALLOWED_PRIVATE_HOSTS") { s.allowed_private_hosts = list(v); applied.push("allowed_private_hosts"); }
    if let Some(v) = var("BRAID_SMTP_HOST") { s.smtp_host = Some(v); applied.push("smtp_host"); }
    if let Some(v) = var("BRAID_SMTP_PORT").and_then(|v| v.parse().ok()) { s.smtp_port = v; applied.push("smtp_port"); }
    if let Some(v) = var("BRAID_SMTP_USERNAME") { s.smtp_username = Some(v); applied.push("smtp_username"); }
    if let Some(v) = var("BRAID_SMTP_FROM") { s.smtp_from = Some(v); applied.push("smtp_from"); }
    if let Some(v) = var("BRAID_SMTP_TLS") { s.smtp_tls = v; applied.push("smtp_tls"); }
    applied
}

pub async fn smtp_password(state: &AppState) -> Option<String> {
    if let Ok(p) = std::env::var("BRAID_SMTP_PASSWORD") {
        return Some(p);
    }
    let ct: Option<serde_json::Value> =
        sqlx::query_scalar("select value from settings where key = 'smtp_password'").fetch_optional(&state.db).await.ok()?;
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, ct?.as_str()?).ok()?;
    state.secrets.decrypt(&bytes).ok()
}

pub async fn set_smtp_password(state: &AppState, password: &str) -> AppResult<()> {
    let ct = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, state.secrets.encrypt(password));
    sqlx::query("insert into settings (key, value) values ('smtp_password', $1) on conflict (key) do update set value = $1")
        .bind(serde_json::Value::String(ct))
        .execute(&state.db)
        .await?;
    Ok(())
}

pub async fn put(state: &AppState, s: &InstanceSettings) -> AppResult<()> {
    sqlx::query("insert into settings (key, value) values ('instance', $1) on conflict (key) do update set value = $1")
        .bind(serde_json::to_value(s).unwrap())
        .execute(&state.db)
        .await?;
    Ok(())
}
