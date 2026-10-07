mod access;
mod admin;
mod audit;
mod auth;
mod chat;
mod crypto;
mod error;
mod gateway;
mod keys;
mod llm;
mod mail;
mod settings;
mod setup;
mod skills;
mod providers;
mod spa;
mod state;
mod upstream;
mod usage;

use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, Method, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use tower_http::trace::TraceLayer;

use crate::{error::AppError, state::AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "braid=info,tower_http=warn,sqlx=warn".into());
    if std::env::var("BRAID_LOG_FORMAT").is_ok_and(|v| v == "pretty") {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt().json().with_env_filter(filter).init();
    }

    let config = state::Config::from_env()?;
    let db = PgPoolOptions::new().max_connections(20).connect(&config.database_url).await?;
    sqlx::migrate!().run(&db).await?;
    let state = AppState::new(config, db).await?;

    let app = Router::new()
        .merge(setup::routes())
        .merge(auth::routes())
        .merge(admin::routes())
        .merge(providers::routes())
        .merge(skills::routes())
        .merge(chat::routes())
        .merge(keys::routes())
        .layer(middleware::from_fn(csrf_guard))
        .merge(gateway::routes())
        .route("/api/health", get(health))
        .fallback(spa::serve)
        .layer(middleware::from_fn(security_headers))
        .layer(axum::extract::DefaultBodyLimit::max(25 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(&state.config.bind).await?;
    tracing::info!("listening on {}", state.config.bind);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn health(State(state): State<AppState>) -> Response {
    match sqlx::query("select 1").execute(&state.db).await {
        Ok(_) => Json(json!({ "status": "ok" })).into_response(),
        Err(e) => AppError::new(axum::http::StatusCode::SERVICE_UNAVAILABLE, "db_down", e.to_string()).into_response(),
    }
}

/// Cookie-authenticated mutations must carry a custom header. Browsers cannot add one
/// cross-site without a CORS preflight, which this server never approves.
async fn csrf_guard(req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && !req.headers().contains_key("x-braid-csrf") {
        return AppError::forbidden("Missing CSRF header").into_response();
    }
    next.run(req).await
}

async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    // script-src lives in the page's CSP meta tag (SvelteKit adds hashes for its inline bootstrap).
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("frame-ancestors 'none'; base-uri 'self'; object-src 'none'; form-action 'self'"),
    );
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("strict-origin-when-cross-origin"));
    res
}
