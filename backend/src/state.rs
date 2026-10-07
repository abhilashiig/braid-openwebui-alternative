use std::{
    collections::HashMap,
    env,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Context;
use axum::http::HeaderMap;
use sqlx::PgPool;

use crate::crypto::Secrets;

pub struct Config {
    pub database_url: String,
    pub bind: String,
    pub public_url: String,
    pub secure_cookies: bool,
    pub trust_proxy: bool,
    pub setup_token: Option<String>,
    pub master_key: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let public_url = env::var("BRAID_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3000".into());
        Ok(Self {
            database_url: env::var("DATABASE_URL").context("DATABASE_URL is required")?,
            bind: env::var("BRAID_BIND").unwrap_or_else(|_| "0.0.0.0:3000".into()),
            secure_cookies: env::var("BRAID_SECURE_COOKIES")
                .map(|v| v == "true" || v == "1")
                .unwrap_or_else(|_| public_url.starts_with("https://")),
            trust_proxy: env::var("BRAID_TRUST_PROXY").is_ok_and(|v| v == "true" || v == "1"),
            setup_token: env::var("BRAID_SETUP_TOKEN").ok().filter(|s| !s.is_empty()),
            master_key: env::var("BRAID_MASTER_KEY")
                .context("BRAID_MASTER_KEY is required (generate with: openssl rand -base64 32)")?,
            public_url: public_url.trim_end_matches('/').to_string(),
        })
    }
}

pub struct Inner {
    pub db: PgPool,
    pub config: Config,
    pub secrets: Secrets,
    pub http: reqwest::Client,
    /// Present only while the install has no users.
    pub setup_token: Mutex<Option<String>>,
    pub login_failures: Mutex<HashMap<String, (u32, std::time::Instant)>>,
}

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub async fn new(config: Config, db: PgPool) -> anyhow::Result<Self> {
        let secrets = Secrets::from_master_key(&config.master_key)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("braid/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let has_users: bool = sqlx::query_scalar("select exists(select 1 from users)").fetch_one(&db).await?;
        let setup_token = (!has_users).then(|| {
            let token = config.setup_token.clone().unwrap_or_else(|| crate::crypto::random_token(18));
            tracing::warn!("No admin yet. Open {}/setup and use setup token: {token}", config.public_url);
            token
        });
        Ok(Self(Arc::new(Inner {
            db,
            config,
            secrets,
            http,
            setup_token: Mutex::new(setup_token),
            login_failures: Mutex::new(HashMap::new()),
        })))
    }

    pub fn client_ip(&self, headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<String> {
        if self.config.trust_proxy {
            if let Some(ip) = headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(',').next())
                .and_then(|v| v.trim().parse::<IpAddr>().ok())
            {
                return Some(ip.to_string());
            }
        }
        peer.map(|p| p.ip().to_string())
    }
}
