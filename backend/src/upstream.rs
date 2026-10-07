//! Talking to configured providers: auth headers, key choice/failover, SSRF guard, model listing.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::http::StatusCode;
use reqwest::RequestBuilder;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    settings,
    state::AppState,
};

#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct Provider {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub api_format: String,
    pub base_url: String,
    pub headers: Value,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub key_strategy: String,
    pub timeout_secs: i32,
    pub max_retries: i32,
    pub enabled: bool,
}

pub const PROVIDER_COLS: &str =
    "id, name, slug, api_format, base_url, headers, organization, project, key_strategy, timeout_secs, max_retries, enabled";

pub async fn load_provider(state: &AppState, id: Uuid) -> AppResult<Provider> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("select {PROVIDER_COLS} from providers where id = $1")))
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Provider"))
}

pub struct ProviderKey {
    pub id: Uuid,
    pub secret: String,
}

static ROUND_ROBIN: std::sync::LazyLock<Mutex<HashMap<Uuid, &'static AtomicUsize>>> =
    std::sync::LazyLock::new(Default::default);

/// Enabled keys in the order they should be tried: healthy keys first, rotated for round-robin.
pub async fn ordered_keys(state: &AppState, p: &Provider) -> AppResult<Vec<ProviderKey>> {
    let rows: Vec<(Uuid, Vec<u8>, bool)> = sqlx::query_as(
        "select id, ciphertext, healthy from provider_keys where provider_id = $1 and enabled
         order by healthy desc, position, created_at",
    )
    .bind(p.id)
    .fetch_all(&state.db)
    .await?;
    let healthy = rows.iter().filter(|r| r.2).count();
    let mut keys = rows
        .into_iter()
        .map(|(id, ct, _)| Ok(ProviderKey { id, secret: state.secrets.decrypt(&ct)? }))
        .collect::<anyhow::Result<Vec<_>>>()?;
    if p.key_strategy == "round_robin" && healthy > 1 {
        let counter = *ROUND_ROBIN
            .lock()
            .unwrap()
            .entry(p.id)
            .or_insert_with(|| Box::leak(Box::new(AtomicUsize::new(0))));
        keys[..healthy].rotate_left(counter.fetch_add(1, Ordering::Relaxed) % healthy);
    }
    Ok(keys)
}

pub async fn mark_key(state: &AppState, key_id: Uuid, healthy: bool, error: Option<String>) {
    let changed = sqlx::query_scalar::<_, Uuid>(
        "update provider_keys set healthy = $2, last_error = $3 where id = $1 and healthy <> $2 returning provider_id",
    )
    .bind(key_id)
    .bind(healthy)
    .bind(&error)
    .fetch_optional(&state.db)
    .await;
    if let Ok(Some(provider_id)) = changed {
        if !healthy {
            tracing::warn!(%key_id, %provider_id, "provider key marked unhealthy: {}", error.as_deref().unwrap_or(""));
            let _ = sqlx::query(
                "insert into audit_log (action, target_type, target_id, details)
                 values ('provider_key.unhealthy', 'provider_key', $1, jsonb_build_object('provider_id', $2::text, 'error', $3::text))",
            )
            .bind(key_id.to_string())
            .bind(provider_id)
            .bind(error)
            .execute(&state.db)
            .await;
        }
    }
}

pub fn url(p: &Provider, path: &str) -> String {
    format!("{}/{}", p.base_url.trim_end_matches('/'), path.trim_start_matches('/'))
}

pub fn authed(p: &Provider, req: RequestBuilder, key: &str) -> RequestBuilder {
    let mut req = if p.api_format == "anthropic" {
        let r = req.header("anthropic-version", "2023-06-01");
        if key.is_empty() { r } else { r.header("x-api-key", key) }
    } else {
        let mut r = if key.is_empty() { req } else { req.bearer_auth(key) };
        if let Some(org) = p.organization.as_deref().filter(|s| !s.is_empty()) {
            r = r.header("OpenAI-Organization", org);
        }
        if let Some(project) = p.project.as_deref().filter(|s| !s.is_empty()) {
            r = r.header("OpenAI-Project", project);
        }
        r
    };
    if let Some(h) = p.headers.as_object() {
        for (k, v) in h {
            if let Some(v) = v.as_str() {
                req = req.header(k, v);
            }
        }
    }
    req.timeout(Duration::from_secs(p.timeout_secs.max(1) as u64))
}

pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64 // CGNAT 100.64/10
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique local
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link local
                || v6.to_ipv4_mapped().is_some_and(|v4| is_private(IpAddr::V4(v4)))
        }
    }
}

/// Rejects URLs that resolve to private, loopback or link-local addresses unless the host is allow-listed.
pub async fn guard_url(state: &AppState, raw: &str) -> AppResult<()> {
    let parsed = reqwest::Url::parse(raw).map_err(|_| AppError::bad_request(format!("Invalid URL: {raw}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::bad_request("URL must start with http:// or https://"));
    }
    let host = parsed.host_str().ok_or_else(|| AppError::bad_request("URL has no host"))?.to_string();
    let allowed = settings::get(state).await?.allowed_private_hosts;
    if allowed.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
        return Ok(());
    }
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addrs = tokio::net::lookup_host((host.trim_matches(|c| c == '[' || c == ']'), port))
        .await
        .map_err(|e| AppError::bad_request(format!("Cannot resolve {host}: {e}")))?;
    for a in addrs {
        if is_private(a.ip()) {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "private_address",
                format!("{host} is a private network address. Allow it as a local server to use it."),
            ));
        }
    }
    Ok(())
}

pub fn host_of(raw: &str) -> Option<String> {
    reqwest::Url::parse(raw).ok().and_then(|u| u.host_str().map(str::to_string))
}

#[derive(Serialize)]
pub struct TestResult {
    pub ok: bool,
    /// ok | auth | network | error | no_key
    pub status: &'static str,
    pub message: String,
    pub model_count: Option<usize>,
}

/// Fetches the upstream model list with the first working key. Auth failures mark that key unhealthy.
pub async fn list_models(state: &AppState, p: &Provider) -> Result<Vec<String>, TestResult> {
    guard_url(state, &p.base_url).await.map_err(|e| TestResult {
        ok: false,
        status: "network",
        message: e.message,
        model_count: None,
    })?;
    let mut keys = ordered_keys(state, p).await.map_err(|e| TestResult {
        ok: false,
        status: "error",
        message: e.message,
        model_count: None,
    })?;
    if keys.is_empty() {
        // Local servers (Ollama, vLLM) usually need no key.
        keys.push(ProviderKey { id: Uuid::nil(), secret: String::new() });
    }
    let mut last = None;
    for key in keys {
        let mut url = url(p, "models");
        if p.api_format == "anthropic" {
            url.push_str("?limit=1000");
        }
        let res = authed(p, state.http.get(url), &key.secret).timeout(Duration::from_secs(20)).send().await;
        let res = match res {
            Ok(r) => r,
            Err(e) => {
                last = Some(TestResult { ok: false, status: "network", message: network_message(&e), model_count: None });
                continue;
            }
        };
        let status = res.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            let body = res.text().await.unwrap_or_default();
            if !key.id.is_nil() {
                mark_key(state, key.id, false, Some(format!("HTTP {status}: {}", truncate(&body, 300)))).await;
            }
            last = Some(TestResult {
                ok: false,
                status: "auth",
                message: if key.id.is_nil() {
                    "The provider requires an API key. Add one.".into()
                } else {
                    format!("The provider rejected the API key (HTTP {})", status.as_u16())
                },
                model_count: None,
            });
            continue;
        }
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(TestResult {
                ok: false,
                status: "error",
                message: format!("HTTP {}: {}", status.as_u16(), truncate(&body, 300)),
                model_count: None,
            });
        }
        if !key.id.is_nil() {
            mark_key(state, key.id, true, None).await;
        }
        let body: Value = res.json().await.map_err(|e| TestResult {
            ok: false,
            status: "error",
            message: format!("Unexpected response: {e}"),
            model_count: None,
        })?;
        let mut ids: Vec<String> = body["data"]
            .as_array()
            .or_else(|| body["models"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|m| m["id"].as_str().or_else(|| m["name"].as_str()).map(str::to_string))
            .collect();
        ids.sort();
        ids.dedup();
        return Ok(ids);
    }
    Err(last.unwrap())
}

pub fn network_message(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "The provider did not respond in time".into()
    } else if e.is_connect() {
        "Could not connect to the provider".into()
    } else {
        format!("Network error: {e}")
    }
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n).collect::<String>() + "…" }
}

/// Default capabilities by model-id pattern.
/// ponytail: substring heuristic over well-known families; replace with a maintained model table if it drifts.
pub struct Known {
    pub vision: bool,
    pub tools: bool,
    pub reasoning: bool,
    pub context_window: Option<i32>,
}

pub fn known_model(id: &str) -> Known {
    let id = id.to_lowercase();
    let has = |s: &str| id.contains(s);
    let reasoning = has("claude-opus-4")
        || has("claude-sonnet-4")
        || has("claude-3-7")
        || has("claude-haiku-4")
        || has("claude-fable")
        || has("claude-opus-5")
        || has("claude-sonnet-5")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || has("gpt-5")
        || has("deepseek-r1")
        || has("reasoner")
        || has("qwq")
        || has("thinking");
    let claude = has("claude");
    let vision = claude
        || has("gpt-4o")
        || has("gpt-4.1")
        || has("gpt-5")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || has("gemini")
        || has("vision")
        || has("llava")
        || has("pixtral")
        || has("-vl");
    let embedding = has("embed") || has("whisper") || has("tts") || has("dall-e") || has("moderation");
    let tools = !embedding
        && (claude
            || has("gpt-4")
            || has("gpt-5")
            || has("gpt-3.5")
            || id.starts_with("o3")
            || id.starts_with("o4")
            || has("gemini")
            || has("mistral")
            || has("llama-3")
            || has("llama3")
            || has("qwen")
            || has("deepseek-chat")
            || has("command-r"));
    let context_window = if claude {
        Some(200_000)
    } else if has("gpt-4.1") {
        Some(1_047_576)
    } else if has("gpt-5") {
        Some(400_000)
    } else if has("gpt-4o") || id.starts_with("o") && id.len() > 1 && id.as_bytes()[1].is_ascii_digit() {
        Some(128_000)
    } else {
        None
    };
    Known { vision, tools, reasoning, context_window }
}
