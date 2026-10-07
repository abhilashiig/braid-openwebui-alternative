//! Admin endpoints for users, invitations, groups, group grants, settings and the audit log.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post, put},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    access, audit,
    auth::{self, Admin},
    crypto,
    error::{AppError, AppResult},
    settings::{self, InstanceSettings},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/users", get(list_users))
        .route("/api/admin/users/{id}", get(get_user).patch(update_user).delete(delete_user))
        .route("/api/admin/users/{id}/groups", put(set_user_groups))
        .route("/api/admin/users/{id}/reset-link", post(reset_link))
        .route("/api/admin/invitations", get(list_invitations).post(invite))
        .route("/api/admin/invitations/{id}", delete(revoke_invitation))
        .route("/api/admin/invitations/{id}/resend", post(resend_invitation))
        .route("/api/admin/groups", get(list_groups).post(create_group))
        .route("/api/admin/groups/{id}", get(get_group).patch(update_group).delete(delete_group))
        .route("/api/admin/groups/{id}/members", post(add_members))
        .route("/api/admin/groups/{id}/members/{user_id}", delete(remove_member))
        .route("/api/admin/groups/{id}/grants", post(grant_to_group))
        .route("/api/admin/grants/{id}", delete(delete_grant))
        .route("/api/admin/settings", get(get_settings).put(put_settings))
        .route("/api/admin/audit", get(audit_log))
        .route("/api/admin/usage", get(usage_report))
        .route("/api/admin/usage.csv", get(usage_csv))
}

#[derive(Serialize, sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    email: String,
    name: String,
    role: String,
    status: String,
    allow_api_keys: Option<bool>,
    created_at: DateTime<Utc>,
    last_active_at: Option<DateTime<Utc>>,
    groups: Vec<String>,
    group_ids: Vec<Uuid>,
}

const USER_SELECT: &str = "select u.id, u.email, u.name, u.role, u.status, u.allow_api_keys, u.created_at, u.last_active_at,
    array(select g.name from group_members m join groups g on g.id = m.group_id where m.user_id = u.id order by g.name) as groups,
    array(select m.group_id from group_members m where m.user_id = u.id) as group_ids
    from users u";

#[derive(Deserialize)]
struct UserFilter {
    q: Option<String>,
    role: Option<String>,
    status: Option<String>,
    group_id: Option<Uuid>,
}

async fn list_users(State(state): State<AppState>, _: Admin, Query(f): Query<UserFilter>) -> AppResult<Json<Vec<UserRow>>> {
    let rows: Vec<UserRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{USER_SELECT}
         where u.status <> 'deleted'
           and ($1::text is null or u.email ilike '%' || $1 || '%' or u.name ilike '%' || $1 || '%')
           and ($2::text is null or u.role = $2)
           and ($3::text is null or u.status = $3)
           and ($4::uuid is null or exists(select 1 from group_members m where m.user_id = u.id and m.group_id = $4))
         order by u.name"
    )))
    .bind(f.q.filter(|s| !s.is_empty()))
    .bind(f.role.filter(|s| !s.is_empty()))
    .bind(f.status.filter(|s| !s.is_empty()))
    .bind(f.group_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

async fn get_user(State(state): State<AppState>, _: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let user: UserRow = sqlx::query_as(sqlx::AssertSqlSafe(format!("{USER_SELECT} where u.id = $1")))
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("User"))?;
    let access = access::models_for_user(&state, id, true).await?;
    let grants: Vec<(Uuid, Uuid)> =
        sqlx::query_as("select id, model_id from grants where user_id = $1 and model_id is not null")
            .bind(id)
            .fetch_all(&state.db)
            .await?;
    let models: Vec<Value> = access
        .iter()
        .map(|a| {
            json!({
                "id": a.model.id, "display_name": a.model.display_name, "name": a.model.name,
                "provider_name": a.model.provider_name, "enabled": a.model.enabled, "visibility": a.model.visibility,
                "allowed": a.allowed(), "is_public": a.is_public, "direct": a.direct, "via_groups": a.via_groups,
                "direct_grant_id": grants.iter().find(|g| g.1 == a.model.id).map(|g| g.0),
            })
        })
        .collect();
    Ok(Json(json!({ "user": user, "models": models })))
}

async fn active_admins_except(state: &AppState, id: Uuid) -> AppResult<i64> {
    Ok(sqlx::query_scalar("select count(*) from users where role = 'admin' and status = 'active' and id <> $1")
        .bind(id)
        .fetch_one(&state.db)
        .await?)
}

#[derive(Deserialize)]
struct UserPatch {
    role: Option<String>,
    status: Option<String>,
    name: Option<String>,
    /// Send `"allow_api_keys": null` to inherit from groups/global setting.
    #[serde(default, deserialize_with = "double_option")]
    allow_api_keys: Option<Option<bool>>,
}

fn double_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

async fn update_user(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(p): Json<UserPatch>,
) -> AppResult<Json<Value>> {
    let (role, status): (String, String) = sqlx::query_as("select role, status from users where id = $1 and status <> 'deleted'")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("User"))?;
    if let Some(r) = &p.role {
        if !matches!(r.as_str(), "admin" | "user") {
            return Err(AppError::bad_request("Role must be admin or user"));
        }
    }
    if let Some(s) = &p.status {
        if !matches!(s.as_str(), "active" | "deactivated") {
            return Err(AppError::bad_request("Status must be active or deactivated"));
        }
    }
    let loses_admin = role == "admin"
        && status == "active"
        && (p.role.as_deref() == Some("user") || p.status.as_deref() == Some("deactivated"));
    if loses_admin && active_admins_except(&state, id).await? == 0 {
        return Err(AppError::conflict("This is the last active admin. Promote someone else first."));
    }
    let name = p.name.as_deref().map(auth::require_name).transpose()?;
    sqlx::query(
        "update users set role = coalesce($2, role), status = coalesce($3, status), name = coalesce($4, name),
         allow_api_keys = case when $5 then $6 else allow_api_keys end where id = $1",
    )
    .bind(id)
    .bind(&p.role)
    .bind(&p.status)
    .bind(name)
    .bind(p.allow_api_keys.is_some())
    .bind(p.allow_api_keys.flatten())
    .execute(&state.db)
    .await?;
    if p.status.as_deref() == Some("deactivated") {
        sqlx::query("delete from sessions where user_id = $1").bind(id).execute(&state.db).await?;
    }
    audit::record(
        &state,
        &admin.0,
        "user.updated",
        "user",
        id,
        json!({ "role": p.role, "status": p.status, "name": name, "allow_api_keys": p.allow_api_keys }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct DeleteQ {
    #[serde(default)]
    erase_chats: bool,
}

/// Erasing removes the row (chats cascade). Keeping chats anonymizes the account and marks it deleted.
async fn delete_user(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Query(q): Query<DeleteQ>,
) -> AppResult<Json<Value>> {
    if id == admin.0.id {
        return Err(AppError::bad_request("You cannot delete your own account"));
    }
    let (email, role): (String, String) = sqlx::query_as("select email, role from users where id = $1 and status <> 'deleted'")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("User"))?;
    if role == "admin" && active_admins_except(&state, id).await? == 0 {
        return Err(AppError::conflict("This is the last active admin"));
    }
    if q.erase_chats {
        sqlx::query("delete from users where id = $1").bind(id).execute(&state.db).await?;
    } else {
        let mut tx = state.db.begin().await?;
        sqlx::query(
            "update users set status = 'deleted', email = 'deleted-' || id || '@deleted.invalid', password_hash = null,
             role = 'user' where id = $1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("delete from sessions where user_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("update api_keys set revoked_at = now() where user_id = $1 and revoked_at is null")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("delete from group_members where user_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("delete from grants where user_id = $1").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
    }
    audit::record(&state, &admin.0, "user.deleted", "user", id, json!({ "email": email, "erase_chats": q.erase_chats })).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct GroupIds {
    group_ids: Vec<Uuid>,
}

async fn set_user_groups(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(req): Json<GroupIds>,
) -> AppResult<Json<Value>> {
    let mut tx = state.db.begin().await?;
    sqlx::query("delete from group_members where user_id = $1").bind(id).execute(&mut *tx).await?;
    sqlx::query(
        "insert into group_members (group_id, user_id) select g.id, $2 from groups g where g.id = any($1) and not g.is_everyone",
    )
    .bind(&req.group_ids)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    audit::record(&state, &admin.0, "user.groups_set", "user", id, json!({ "group_ids": req.group_ids })).await;
    Ok(Json(json!({ "ok": true })))
}

/// Forced reset: signs the user out everywhere and returns a one-time link to set a new password.
async fn reset_link(State(state): State<AppState>, admin: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let token = crypto::random_token(32);
    let mut tx = state.db.begin().await?;
    let exists: bool = sqlx::query_scalar("update users set must_reset_password = true where id = $1 and status = 'active' returning true")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);
    if !exists {
        return Err(AppError::not_found("Active user"));
    }
    sqlx::query("insert into password_resets (token_hash, user_id, expires_at) values ($1, $2, $3)")
        .bind(crypto::sha256(&token))
        .bind(id)
        .bind(Utc::now() + Duration::hours(24))
        .execute(&mut *tx)
        .await?;
    if id != admin.0.id {
        sqlx::query("delete from sessions where user_id = $1").bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    audit::record(&state, &admin.0, "user.password_reset_forced", "user", id, json!({})).await;
    Ok(Json(json!({ "link": format!("{}/reset/{token}", state.config.public_url), "expires_in_hours": 24 })))
}

#[derive(Serialize, sqlx::FromRow)]
struct InvitationRow {
    id: Uuid,
    email: String,
    role: String,
    group_ids: Vec<Uuid>,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    invited_by_name: Option<String>,
}

async fn list_invitations(State(state): State<AppState>, _: Admin) -> AppResult<Json<Vec<InvitationRow>>> {
    Ok(Json(
        sqlx::query_as(
            "select i.id, i.email, i.role, i.group_ids, i.expires_at, i.created_at, u.name as invited_by_name
             from invitations i left join users u on u.id = i.invited_by
             where i.accepted_at is null and i.revoked_at is null order by i.created_at desc",
        )
        .fetch_all(&state.db)
        .await?,
    ))
}

#[derive(Deserialize)]
struct InviteReq {
    emails: Vec<String>,
    role: String,
    #[serde(default)]
    group_ids: Vec<Uuid>,
}

async fn invite(State(state): State<AppState>, admin: Admin, Json(req): Json<InviteReq>) -> AppResult<Json<Value>> {
    if !matches!(req.role.as_str(), "admin" | "user") {
        return Err(AppError::bad_request("Role must be admin or user"));
    }
    if req.emails.is_empty() || req.emails.len() > 500 {
        return Err(AppError::bad_request("Provide between 1 and 500 emails"));
    }
    let ttl = settings::get(&state).await?.invite_ttl_days.clamp(1, 90);
    let mut invited = vec![];
    let mut errors = vec![];
    for raw in &req.emails {
        let email = match auth::normalize_email(raw) {
            Ok(e) => e,
            Err(e) => {
                errors.push(json!({ "email": raw, "error": e.message }));
                continue;
            }
        };
        let exists: bool = sqlx::query_scalar("select exists(select 1 from users where lower(email) = $1)")
            .bind(&email)
            .fetch_one(&state.db)
            .await?;
        if exists {
            errors.push(json!({ "email": email, "error": "Already has an account" }));
            continue;
        }
        // A new invitation for the same email replaces any pending one.
        sqlx::query("update invitations set revoked_at = now() where lower(email) = $1 and accepted_at is null and revoked_at is null")
            .bind(&email)
            .execute(&state.db)
            .await?;
        let token = crypto::random_token(32);
        let id: Uuid = sqlx::query_scalar(
            "insert into invitations (email, role, group_ids, token_hash, invited_by, expires_at)
             values ($1, $2, $3, $4, $5, $6) returning id",
        )
        .bind(&email)
        .bind(&req.role)
        .bind(&req.group_ids)
        .bind(crypto::sha256(&token))
        .bind(admin.0.id)
        .bind(Utc::now() + Duration::days(ttl))
        .fetch_one(&state.db)
        .await?;
        invited.push(json!({ "id": id, "email": email, "link": format!("{}/invite/{token}", state.config.public_url) }));
    }
    audit::record(
        &state,
        &admin.0,
        "invitation.created",
        "invitation",
        "",
        json!({ "emails": invited.iter().map(|i| i["email"].clone()).collect::<Vec<_>>(), "role": req.role, "group_ids": req.group_ids }),
    )
    .await;
    Ok(Json(json!({ "invited": invited, "errors": errors })))
}

async fn resend_invitation(State(state): State<AppState>, admin: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let ttl = settings::get(&state).await?.invite_ttl_days.clamp(1, 90);
    let token = crypto::random_token(32);
    let email: String = sqlx::query_scalar(
        "update invitations set token_hash = $2, expires_at = $3 where id = $1 and accepted_at is null and revoked_at is null
         returning email",
    )
    .bind(id)
    .bind(crypto::sha256(&token))
    .bind(Utc::now() + Duration::days(ttl))
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("Pending invitation"))?;
    audit::record(&state, &admin.0, "invitation.resent", "invitation", id, json!({ "email": email })).await;
    Ok(Json(json!({ "email": email, "link": format!("{}/invite/{token}", state.config.public_url) })))
}

async fn revoke_invitation(State(state): State<AppState>, admin: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let email: String = sqlx::query_scalar(
        "update invitations set revoked_at = now() where id = $1 and accepted_at is null and revoked_at is null returning email",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("Pending invitation"))?;
    audit::record(&state, &admin.0, "invitation.revoked", "invitation", id, json!({ "email": email })).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize, sqlx::FromRow)]
struct GroupRow {
    id: Uuid,
    name: String,
    description: String,
    is_everyone: bool,
    allow_api_keys: Option<bool>,
    member_count: i64,
    model_count: i64,
}

const GROUP_SELECT: &str = "select g.id, g.name, g.description, g.is_everyone, g.allow_api_keys,
    case when g.is_everyone then (select count(*) from users where status = 'active')
         else (select count(*) from group_members m join users u on u.id = m.user_id where m.group_id = g.id and u.status = 'active') end as member_count,
    (select count(*) from grants where group_id = g.id and model_id is not null) as model_count
    from groups g";

async fn list_groups(State(state): State<AppState>, _: Admin) -> AppResult<Json<Vec<GroupRow>>> {
    Ok(Json(sqlx::query_as(sqlx::AssertSqlSafe(format!("{GROUP_SELECT} order by g.is_everyone desc, g.name"))).fetch_all(&state.db).await?))
}

async fn get_group(State(state): State<AppState>, _: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let group: GroupRow = sqlx::query_as(sqlx::AssertSqlSafe(format!("{GROUP_SELECT} where g.id = $1")))
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Group"))?;
    let members: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "select u.id, u.name, u.email, u.status from users u
         where u.status <> 'deleted' and ($2 or u.id in (select user_id from group_members where group_id = $1))
         order by u.name",
    )
    .bind(id)
    .bind(group.is_everyone)
    .fetch_all(&state.db)
    .await?;
    let grants: Vec<(Uuid, Uuid, String, String, String)> = sqlx::query_as(
        "select gr.id, m.id, m.display_name, m.name, p.name from grants gr join models m on m.id = gr.model_id
         join providers p on p.id = m.provider_id where gr.group_id = $1 order by m.display_name",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "group": group,
        "members": members.iter().map(|m| json!({ "id": m.0, "name": m.1, "email": m.2, "status": m.3 })).collect::<Vec<_>>(),
        "grants": grants.iter().map(|g| json!({ "grant_id": g.0, "model_id": g.1, "display_name": g.2, "name": g.3, "provider_name": g.4 })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct GroupReq {
    name: String,
    #[serde(default)]
    description: String,
}

async fn create_group(State(state): State<AppState>, admin: Admin, Json(req): Json<GroupReq>) -> AppResult<Json<Value>> {
    let name = auth::require_name(&req.name)?;
    let id: Uuid = sqlx::query_scalar("insert into groups (name, description) values ($1, $2) returning id")
        .bind(name)
        .bind(req.description.trim())
        .fetch_one(&state.db)
        .await
        .map_err(|e| match AppError::from(e) {
            e if e.code == "conflict" => AppError::conflict("A group with this name already exists"),
            e => e,
        })?;
    audit::record(&state, &admin.0, "group.created", "group", id, json!({ "name": name })).await;
    Ok(Json(json!({ "id": id })))
}

#[derive(Deserialize)]
struct GroupPatch {
    name: Option<String>,
    description: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    allow_api_keys: Option<Option<bool>>,
}

async fn update_group(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(p): Json<GroupPatch>,
) -> AppResult<Json<Value>> {
    let name = p.name.as_deref().map(auth::require_name).transpose()?;
    let n = sqlx::query(
        "update groups set name = coalesce($2, name), description = coalesce($3, description),
         allow_api_keys = case when $4 then $5 else allow_api_keys end where id = $1",
    )
    .bind(id)
    .bind(name)
    .bind(p.description.as_deref().map(str::trim))
    .bind(p.allow_api_keys.is_some())
    .bind(p.allow_api_keys.flatten())
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::not_found("Group"));
    }
    audit::record(
        &state,
        &admin.0,
        "group.updated",
        "group",
        id,
        json!({ "name": name, "description": p.description, "allow_api_keys": p.allow_api_keys }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_group(State(state): State<AppState>, admin: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let name: String = sqlx::query_scalar("delete from groups where id = $1 and not is_everyone returning name")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Group (the Everyone group cannot be deleted)"))?;
    audit::record(&state, &admin.0, "group.deleted", "group", id, json!({ "name": name })).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct UserIds {
    user_ids: Vec<Uuid>,
}

async fn add_members(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(req): Json<UserIds>,
) -> AppResult<Json<Value>> {
    let is_everyone: bool = sqlx::query_scalar("select is_everyone from groups where id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Group"))?;
    if is_everyone {
        return Err(AppError::bad_request("Everyone always contains all active users"));
    }
    sqlx::query("insert into group_members (group_id, user_id) select $1, unnest($2::uuid[]) on conflict do nothing")
        .bind(id)
        .bind(&req.user_ids)
        .execute(&state.db)
        .await?;
    audit::record(&state, &admin.0, "group.members_added", "group", id, json!({ "user_ids": req.user_ids })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn remove_member(
    State(state): State<AppState>,
    admin: Admin,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<Value>> {
    sqlx::query("delete from group_members where group_id = $1 and user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    audit::record(&state, &admin.0, "group.member_removed", "group", id, json!({ "user_id": user_id })).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ModelIds {
    model_ids: Vec<Uuid>,
}

/// Bulk grant: several models to one group.
async fn grant_to_group(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(req): Json<ModelIds>,
) -> AppResult<Json<Value>> {
    sqlx::query("insert into grants (model_id, group_id) select unnest($2::uuid[]), $1 on conflict do nothing")
        .bind(id)
        .bind(&req.model_ids)
        .execute(&state.db)
        .await?;
    audit::record(&state, &admin.0, "grant.created", "group", id, json!({ "model_ids": req.model_ids })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_grant(State(state): State<AppState>, admin: Admin, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let row: (Option<Uuid>, Option<String>, Option<Uuid>, Option<Uuid>) =
        sqlx::query_as("delete from grants where id = $1 returning model_id, skill, user_id, group_id")
            .bind(id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| AppError::not_found("Grant"))?;
    audit::record(
        &state,
        &admin.0,
        "grant.deleted",
        "grant",
        id,
        json!({ "model_id": row.0, "skill": row.1, "user_id": row.2, "group_id": row.3 }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

async fn get_settings(State(state): State<AppState>, _: Admin) -> AppResult<Json<InstanceSettings>> {
    Ok(Json(settings::get(&state).await?))
}

async fn put_settings(
    State(state): State<AppState>,
    admin: Admin,
    Json(mut s): Json<InstanceSettings>,
) -> AppResult<Json<Value>> {
    s.name = s.name.trim().to_string();
    if s.name.is_empty() {
        return Err(AppError::bad_request("Instance name is required"));
    }
    if !matches!(s.signup_mode.as_str(), "invite" | "open") {
        return Err(AppError::bad_request("Sign-up mode must be invite or open"));
    }
    s.signup_domains = s.signup_domains.iter().map(|d| d.trim().trim_start_matches('@').to_lowercase()).filter(|d| !d.is_empty()).collect();
    s.allowed_private_hosts = s.allowed_private_hosts.iter().map(|h| h.trim().to_string()).filter(|h| !h.is_empty()).collect();
    s.logo_url = s.logo_url.filter(|u| !u.trim().is_empty());
    if s.logo_url.as_deref().is_some_and(|u| !(u.starts_with("https://") || u.starts_with("data:image/") || u.starts_with('/'))) {
        return Err(AppError::bad_request("Logo must be an https:// URL or an uploaded image"));
    }
    s.max_api_keys_per_user = s.max_api_keys_per_user.clamp(0, 100);
    s.max_tool_calls = s.max_tool_calls.clamp(1, 20);
    let before = settings::get(&state).await?;
    settings::put(&state, &s).await?;
    let (b, a) = (serde_json::to_value(&before).unwrap(), serde_json::to_value(&s).unwrap());
    let changed: serde_json::Map<String, Value> = a
        .as_object()
        .unwrap()
        .iter()
        .filter(|(k, v)| b.get(k.as_str()) != Some(v))
        .map(|(k, v)| (k.clone(), if k == "logo_url" { json!("(changed)") } else { v.clone() }))
        .collect();
    audit::record(&state, &admin.0, "settings.updated", "settings", "instance", Value::Object(changed)).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct AuditQ {
    before: Option<i64>,
    action: Option<String>,
}

async fn audit_log(State(state): State<AppState>, _: Admin, Query(q): Query<AuditQ>) -> AppResult<Json<Value>> {
    let rows: Vec<(i64, DateTime<Utc>, Option<String>, String, String, Option<String>, Value, Option<String>)> = sqlx::query_as(
        "select id, created_at, actor_email, action, target_type, target_id, details, ip from audit_log
         where ($1::bigint is null or id < $1) and ($2::text is null or action like $2 || '%')
         order by id desc limit 100",
    )
    .bind(q.before)
    .bind(q.action.filter(|s| !s.is_empty()))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(rows
        .into_iter()
        .map(|r| json!({ "id": r.0, "created_at": r.1, "actor_email": r.2, "action": r.3, "target_type": r.4,
                         "target_id": r.5, "details": r.6, "ip": r.7 }))
        .collect::<Vec<_>>())))
}

#[derive(Deserialize)]
struct UsageQ {
    days: Option<i32>,
    by: Option<String>,
    source: Option<String>,
}

/// Usage dashboard: totals grouped by day, user, model, provider or API key (NFR-AUD-02, FR-GW-08).
async fn usage_report(State(state): State<AppState>, _: Admin, Query(q): Query<UsageQ>) -> AppResult<Json<Value>> {
    let (key, label) = match q.by.as_deref().unwrap_or("day") {
        "user" => ("l.user_id::text", "coalesce(max(u.name) || ' <' || max(u.email) || '>', '(deleted user)')"),
        "model" => ("l.model_name", "coalesce(l.model_name, '(unknown)')"),
        "provider" => ("l.provider_name", "coalesce(l.provider_name, '(unknown)')"),
        "key" => ("l.api_key_id::text", "coalesce(max(k.name) || ' (' || max(k.prefix) || '…, ' || max(u.email) || ')', 'Chat (no key)')"),
        _ => ("l.created_at::date::text", "l.created_at::date::text"),
    };
    let order = if q.by.as_deref().unwrap_or("day") == "day" { "1 desc" } else { "requests desc" };
    let rows: Vec<(Option<String>, Option<String>, i64, i64, Option<i64>, Option<i64>, Option<i64>, Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<f64>)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select {key} as k, {label} as label, count(*) as requests, count(*) filter (where l.status >= 400) as errors,
                sum(l.input_tokens)::bigint, sum(l.output_tokens)::bigint, sum(l.cached_tokens)::bigint, sum(l.cost),
                avg(l.output_tps)::float8, avg(l.ttft_ms)::float8, avg(l.prefill_tps)::float8,
                case when sum(l.input_tokens) filter (where l.cached_tokens is not null) > 0
                     then sum(l.cached_tokens)::float8 / sum(l.input_tokens) filter (where l.cached_tokens is not null) end
             from usage_log l left join users u on u.id = l.user_id left join api_keys k on k.id = l.api_key_id
             where l.created_at >= now() - make_interval(days => $1)
               and ($2::text is null or l.source = $2) and l.source <> 'skill'
             group by {key} order by {order} limit 500"
        )))
        .bind(q.days.unwrap_or(30).clamp(1, 366))
        .bind(q.source.filter(|s| s == "chat" || s == "api"))
        .fetch_all(&state.db)
        .await?;
    let searches: i64 = sqlx::query_scalar(
        "select count(*) from usage_log where source = 'skill' and created_at >= now() - make_interval(days => $1)",
    )
    .bind(q.days.unwrap_or(30).clamp(1, 366))
    .fetch_one(&state.db)
    .await?;
    Ok(Json(json!({
        "rows": rows.iter().map(|r| json!({
            "key": r.0, "label": r.1, "requests": r.2, "errors": r.3, "input_tokens": r.4, "output_tokens": r.5,
            "cached_tokens": r.6, "cost": r.7, "avg_output_tps": r.8, "avg_ttft_ms": r.9, "avg_prefill_tps": r.10, "cache_rate": r.11,
        })).collect::<Vec<_>>(),
        "web_searches": searches,
    })))
}

async fn usage_csv(State(state): State<AppState>, _: Admin, Query(q): Query<UsageQ>) -> AppResult<axum::response::Response> {
    use axum::response::IntoResponse;
    let rows: Vec<(DateTime<Utc>, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, i32, Option<i32>, Option<i32>, Option<i32>, Option<f64>, Option<i32>, Option<i32>, Option<f32>, Option<String>)> =
        sqlx::query_as(
            "select l.created_at, l.source, u.email, k.prefix, l.model_name, l.provider_name, l.metrics_source, l.status,
                l.input_tokens, l.output_tokens, l.cached_tokens, l.cost, l.latency_ms, l.ttft_ms, l.output_tps, l.error
             from usage_log l left join users u on u.id = l.user_id left join api_keys k on k.id = l.api_key_id
             where l.created_at >= now() - make_interval(days => $1) order by l.created_at desc limit 100000",
        )
        .bind(q.days.unwrap_or(30).clamp(1, 366))
        .fetch_all(&state.db)
        .await?;
    let esc = |s: &str| {
        // Leading =,+,-,@ would be evaluated as formulas by spreadsheet apps.
        let s = if s.starts_with(['=', '+', '-', '@']) { format!("'{s}") } else { s.to_string() };
        format!("\"{}\"", s.replace('"', "\"\""))
    };
    let o = |v: Option<String>| v.map(|s| esc(&s)).unwrap_or_default();
    let n = |v: Option<String>| v.unwrap_or_default();
    let mut csv = String::from("time,source,user,api_key,model,provider,metrics_source,status,input_tokens,output_tokens,cached_tokens,cost,latency_ms,ttft_ms,output_tps,error\n");
    for r in rows {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            r.0.to_rfc3339(), r.1, o(r.2), o(r.3), o(r.4), o(r.5), o(r.6), r.7,
            n(r.8.map(|v| v.to_string())), n(r.9.map(|v| v.to_string())), n(r.10.map(|v| v.to_string())),
            n(r.11.map(|v| format!("{v:.6}"))), n(r.12.map(|v| v.to_string())), n(r.13.map(|v| v.to_string())),
            n(r.14.map(|v| format!("{v:.1}"))), o(r.15),
        ));
    }
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (axum::http::header::CONTENT_DISPOSITION, "attachment; filename=\"braid-usage.csv\""),
        ],
        csv,
    )
        .into_response())
}
