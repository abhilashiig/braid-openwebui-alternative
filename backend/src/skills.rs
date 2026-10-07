//! Skills are tools the model may call. v1 ships web search (Brave) with an optional page fetch.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post, put},
};
use futures::StreamExt;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    audit,
    auth::Admin,
    crypto,
    error::{AppError, AppResult},
    llm::ToolDef,
    state::AppState,
    upstream,
    usage::UsageRow,
};

pub const WEB_SEARCH: &str = "web_search";
const FETCH_PAGE: &str = "fetch_page";
const SEARCHES_PER_MINUTE: u32 = 20;
const MAX_PAGE_BYTES: usize = 2_000_000;
const MAX_PAGE_CHARS: usize = 20_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub date: Option<String>,
}

pub struct SkillOutput {
    /// What the model sees as the tool result.
    pub for_model: String,
    /// Short label for the UI step, e.g. `Searched: rust 2026`.
    pub summary: String,
    pub sources: Vec<Source>,
    pub error: Option<String>,
}

/// Search provider interface so Tavily, SearXNG or Google PSE can be added later (FR-WEB-07).
trait SearchProvider {
    async fn search(&self, http: &reqwest::Client, query: &str, count: u32, freshness: Option<&str>) -> Result<Vec<Source>, String>;
}

struct Brave {
    key: String,
}

impl SearchProvider for Brave {
    async fn search(&self, http: &reqwest::Client, query: &str, count: u32, freshness: Option<&str>) -> Result<Vec<Source>, String> {
        let mut params = vec![("q", query.to_string()), ("count", count.to_string())];
        if let Some(f) = freshness.and_then(|f| match f {
            "day" => Some("pd"),
            "week" => Some("pw"),
            "month" => Some("pm"),
            "year" => Some("py"),
            _ => None,
        }) {
            params.push(("freshness", f.into()));
        }
        let url = reqwest::Url::parse_with_params("https://api.search.brave.com/res/v1/web/search", &params).map_err(|e| e.to_string())?;
        let res = http
            .get(url)
            .header("X-Subscription-Token", &self.key)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| upstream::network_message(&e))?;
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(match status.as_u16() {
                401 | 403 | 422 => "The Brave Search API key was rejected".into(),
                429 => "Brave Search rate limit reached".into(),
                s => format!("Brave Search returned HTTP {s}: {}", upstream::truncate(&body, 200)),
            });
        }
        let v: Value = res.json().await.map_err(|e| format!("Unexpected Brave response: {e}"))?;
        Ok(v["web"]["results"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| Source {
                title: strip_tags(r["title"].as_str().unwrap_or_default()),
                url: r["url"].as_str().unwrap_or_default().into(),
                snippet: strip_tags(r["description"].as_str().unwrap_or_default()),
                date: r["page_age"].as_str().or_else(|| r["age"].as_str()).map(str::to_string),
            })
            .collect())
    }
}

pub fn tool_defs(skills: &[String]) -> Vec<ToolDef> {
    let mut out = vec![];
    if skills.iter().any(|s| s == WEB_SEARCH) {
        out.push(ToolDef {
            name: WEB_SEARCH.into(),
            description: "Search the web for current information. Returns titles, URLs, snippets and dates. \
                Cite the sources you use in your answer as markdown links."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "count": { "type": "integer", "minimum": 1, "maximum": 20, "description": "Number of results (default 5)" },
                    "freshness": { "type": "string", "enum": ["day", "week", "month", "year"], "description": "Only results from this period" }
                },
                "required": ["query"]
            }),
        });
        out.push(ToolDef {
            name: FETCH_PAGE.into(),
            description: "Fetch a web page found via web_search and return its main text, for deeper answers.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "url": { "type": "string", "description": "http(s) URL to fetch" } },
                "required": ["url"]
            }),
        });
    }
    out
}

/// Skills the user may use: enabled globally, configured, and public or granted to them or their groups.
pub async fn available_for_user(state: &AppState, user_id: Uuid) -> AppResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "select s.name from skills s where s.enabled and s.secret_ciphertext is not null and (s.visibility = 'public' or exists (
            select 1 from grants g where g.skill = s.name and (g.user_id = $1 or g.group_id in (
                select group_id from group_members where user_id = $1 union select id from groups where is_everyone))))",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?)
}

static RATE: LazyLock<Mutex<HashMap<Uuid, (u32, Instant)>>> = LazyLock::new(Default::default);

fn rate_ok(user_id: Uuid) -> bool {
    // ponytail: fixed one-minute window per user, in memory and per node.
    let mut m = RATE.lock().unwrap();
    let e = m.entry(user_id).or_insert((0, Instant::now()));
    if e.1.elapsed() > Duration::from_secs(60) {
        *e = (0, Instant::now());
    }
    e.0 += 1;
    e.0 <= SEARCHES_PER_MINUTE
}

fn failed(summary: String, error: String) -> SkillOutput {
    SkillOutput { for_model: format!("Error: {error}"), summary, sources: vec![], error: Some(error) }
}

pub async fn run(state: &AppState, user_id: Uuid, api_key_id: Option<Uuid>, name: &str, arguments: &str) -> SkillOutput {
    let args: Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!({}));
    match name {
        WEB_SEARCH => {
            let query = args["query"].as_str().unwrap_or_default().trim().to_string();
            let summary = format!("Searched: {query}");
            if query.is_empty() {
                return failed(summary, "query is required".into());
            }
            if !rate_ok(user_id) {
                return failed(summary, "Search rate limit reached; try again in a minute".into());
            }
            let secret: Option<Vec<u8>> =
                sqlx::query_scalar("select secret_ciphertext from skills where name = $1 and enabled")
                    .bind(WEB_SEARCH)
                    .fetch_optional(&state.db)
                    .await
                    .ok()
                    .flatten();
            let Some(key) = secret.and_then(|c| state.secrets.decrypt(&c).ok()) else {
                return failed(summary, "Web search is not configured".into());
            };
            let count = args["count"].as_u64().unwrap_or(5).clamp(1, 20) as u32;
            let started = Instant::now();
            let result = Brave { key }.search(&state.http, &query, count, args["freshness"].as_str()).await;
            let _ = state.usage.try_send(UsageRow {
                source: "skill",
                user_id: Some(user_id),
                api_key_id,
                model_name: Some(WEB_SEARCH.into()),
                provider_name: Some("Brave Search".into()),
                status: if result.is_ok() { 200 } else { 502 },
                error: result.as_ref().err().cloned(),
                latency_ms: Some(started.elapsed().as_millis() as i64),
                ..Default::default()
            });
            match result {
                Ok(sources) => {
                    let listing: Vec<Value> = sources
                        .iter()
                        .enumerate()
                        .map(|(i, s)| json!({ "n": i + 1, "title": s.title, "url": s.url, "snippet": s.snippet, "date": s.date }))
                        .collect();
                    SkillOutput {
                        for_model: json!({
                            "results": listing,
                            "instructions": "Answer using these results and cite each source you rely on as a markdown link [title](url)."
                        })
                        .to_string(),
                        summary,
                        sources,
                        error: None,
                    }
                }
                Err(e) => failed(summary, e),
            }
        }
        FETCH_PAGE => {
            let url = args["url"].as_str().unwrap_or_default().to_string();
            let summary = format!("Read: {url}");
            match fetch_page(&url).await {
                Ok((title, text)) => SkillOutput {
                    for_model: json!({ "url": url, "title": title, "text": text }).to_string(),
                    summary,
                    sources: vec![Source { title: if title.is_empty() { url.clone() } else { title }, url, snippet: String::new(), date: None }],
                    error: None,
                },
                Err(e) => failed(summary, e),
            }
        }
        other => failed(format!("Unknown tool {other}"), format!("Unknown tool {other}")),
    }
}

/// Resolver that refuses private, loopback and link-local addresses, so page fetches cannot reach the
/// internal network even through DNS rebinding or redirects.
struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if addrs.is_empty() || addrs.iter().any(|a| upstream::is_private(a.ip())) {
                return Err(format!("{host} resolves to a private address").into());
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

static FETCH_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .dns_resolver(std::sync::Arc::new(PublicOnly))
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("Mozilla/5.0 (compatible; braid/", env!("CARGO_PKG_VERSION"), ")"))
        .build()
        .expect("fetch client")
});

async fn fetch_page(raw: &str) -> Result<(String, String), String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "Invalid URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only http(s) URLs can be fetched".into());
    }
    // IP literals skip DNS, so check them here.
    if let Some(ip) = url.host_str().and_then(|h| h.trim_matches(|c| c == '[' || c == ']').parse().ok()) {
        if upstream::is_private(ip) {
            return Err("Private addresses cannot be fetched".into());
        }
    }
    let res = FETCH_CLIENT.get(url).send().await.map_err(|e| format!("Fetch failed: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("The page returned HTTP {}", res.status().as_u16()));
    }
    let ctype = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    if !(ctype.contains("html") || ctype.contains("text") || ctype.contains("json") || ctype.is_empty()) {
        return Err(format!("Unsupported content type: {ctype}"));
    }
    let mut body = Vec::new();
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Fetch failed: {e}"))?;
        body.extend_from_slice(&chunk);
        if body.len() > MAX_PAGE_BYTES {
            break;
        }
    }
    let html = String::from_utf8_lossy(&body);
    let title = html
        .find("<title")
        .and_then(|i| html[i..].find('>').map(|j| i + j + 1))
        .and_then(|s| html[s..].find("</title>").map(|e| strip_tags(&html[s..s + e])))
        .unwrap_or_default();
    let text: String = html_to_text(&html).chars().take(MAX_PAGE_CHARS).collect();
    Ok((title, text))
}

// ponytail: tag-stripping text extraction, no readability scoring; swap for a real extractor if answers suffer.
fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    for tag in ["script", "style", "noscript", "svg", "head", "nav", "footer"] {
        loop {
            let lower = s.to_ascii_lowercase();
            let Some(start) = lower.find(&format!("<{tag}")) else { break };
            let end = lower[start..].find(&format!("</{tag}>")).map(|e| start + e + tag.len() + 3).unwrap_or(s.len());
            s.replace_range(start..end, " ");
        }
    }
    strip_tags(&s)
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------- admin ----------

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/skills", get(list))
        .route("/api/admin/skills/{name}", put(update))
        .route("/api/admin/skills/{name}/test", post(test))
        .route("/api/admin/skills/{name}/grants", get(grants).put(set_grants))
}

async fn list(State(state): State<AppState>, _: Admin) -> AppResult<Json<Value>> {
    let rows: Vec<(String, bool, String, Option<String>)> =
        sqlx::query_as("select name, enabled, visibility, secret_last4 from skills order by name").fetch_all(&state.db).await?;
    Ok(Json(json!(rows
        .into_iter()
        .map(|r| json!({
            "name": r.0, "enabled": r.1, "visibility": r.2, "secret_last4": r.3,
            "title": "Web search", "description": "Brave Search API, with optional page fetch", "secret_label": "Brave Search API key",
        }))
        .collect::<Vec<_>>())))
}

#[derive(Deserialize)]
struct SkillUpdate {
    enabled: bool,
    visibility: String,
    /// New secret; omitted or empty keeps the current one.
    secret: Option<String>,
}

async fn update(
    State(state): State<AppState>,
    admin: Admin,
    Path(name): Path<String>,
    Json(req): Json<SkillUpdate>,
) -> AppResult<Json<Value>> {
    if !matches!(req.visibility.as_str(), "public" | "private") {
        return Err(AppError::bad_request("Visibility must be public or private"));
    }
    let secret = req.secret.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let n = sqlx::query(
        "update skills set enabled = $2, visibility = $3,
         secret_ciphertext = coalesce($4, secret_ciphertext), secret_last4 = coalesce($5, secret_last4) where name = $1",
    )
    .bind(&name)
    .bind(req.enabled)
    .bind(&req.visibility)
    .bind(secret.map(|s| state.secrets.encrypt(s)))
    .bind(secret.map(crypto::last4))
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("Skill"));
    }
    audit::record(
        &state,
        &admin.0,
        "skill.updated",
        "skill",
        &name,
        json!({ "enabled": req.enabled, "visibility": req.visibility, "secret_changed": secret.is_some() }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

async fn test(State(state): State<AppState>, admin: Admin, Path(name): Path<String>) -> AppResult<Json<Value>> {
    if name != WEB_SEARCH {
        return Err(AppError::not_found("Skill"));
    }
    let ct: Option<Vec<u8>> = sqlx::query_scalar("select secret_ciphertext from skills where name = $1")
        .bind(&name)
        .fetch_one(&state.db)
        .await?;
    let key = ct.ok_or_else(|| AppError::bad_request("Add the Brave Search API key first"))?;
    let key = state.secrets.decrypt(&key)?;
    Ok(Json(match (Brave { key }).search(&state.http, "Braid self-hosted AI", 3, None).await {
        Ok(r) => json!({ "ok": true, "message": format!("Brave Search works ({} results)", r.len()) }),
        Err(e) => {
            tracing::warn!(admin = %admin.0.email, "web search test failed: {e}");
            json!({ "ok": false, "message": e })
        }
    }))
}

async fn grants(State(state): State<AppState>, _: Admin, Path(name): Path<String>) -> AppResult<Json<Value>> {
    let rows: Vec<(Option<Uuid>, Option<Uuid>)> = sqlx::query_as("select user_id, group_id from grants where skill = $1")
        .bind(&name)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(json!({
        "user_ids": rows.iter().filter_map(|r| r.0).collect::<Vec<_>>(),
        "group_ids": rows.iter().filter_map(|r| r.1).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct GrantSet {
    user_ids: Vec<Uuid>,
    group_ids: Vec<Uuid>,
}

async fn set_grants(
    State(state): State<AppState>,
    admin: Admin,
    Path(name): Path<String>,
    Json(req): Json<GrantSet>,
) -> AppResult<Json<Value>> {
    let mut tx = state.db.begin().await?;
    sqlx::query("delete from grants where skill = $1").bind(&name).execute(&mut *tx).await?;
    sqlx::query("insert into grants (skill, user_id) select $1, unnest($2::uuid[])")
        .bind(&name)
        .bind(&req.user_ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query("insert into grants (skill, group_id) select $1, unnest($2::uuid[])")
        .bind(&name)
        .bind(&req.group_ids)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    audit::record(&state, &admin.0, "skill.grants_set", "skill", &name, json!({ "user_ids": req.user_ids, "group_ids": req.group_ids })).await;
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn extracts_text() {
        let html = "<html><head><title>T &amp; U</title><style>x{}</style></head><body><script>bad()</script><p>Hello&nbsp;<b>world</b></p></body></html>";
        assert_eq!(super::html_to_text(html), "Hello world");
        assert_eq!(super::strip_tags("<b>a</b> &amp; b"), "a & b");
    }
}
