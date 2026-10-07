use serde_json::Value;

use crate::{auth::CurrentUser, state::AppState};

/// Written inline (not batched): admin actions are rare and must not be lost.
pub async fn record(
    state: &AppState,
    actor: &CurrentUser,
    action: &str,
    target_type: &str,
    target_id: impl ToString,
    details: Value,
) {
    let res = sqlx::query(
        "insert into audit_log (actor_id, actor_email, action, target_type, target_id, details, ip)
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(actor.id)
    .bind(&actor.email)
    .bind(action)
    .bind(target_type)
    .bind(target_id.to_string())
    .bind(details)
    .bind(&actor.ip)
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        tracing::error!(action, "failed to write audit log: {e}");
    }
}
