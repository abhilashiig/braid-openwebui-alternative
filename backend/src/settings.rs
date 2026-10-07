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
        }
    }
}

pub async fn get(state: &AppState) -> AppResult<InstanceSettings> {
    let v: Option<serde_json::Value> =
        sqlx::query_scalar("select value from settings where key = 'instance'").fetch_optional(&state.db).await?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
}

pub async fn put(state: &AppState, s: &InstanceSettings) -> AppResult<()> {
    sqlx::query("insert into settings (key, value) values ('instance', $1) on conflict (key) do update set value = $1")
        .bind(serde_json::to_value(s).unwrap())
        .execute(&state.db)
        .await?;
    Ok(())
}
