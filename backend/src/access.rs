//! Who can use which model: public, direct grant, or a grant to any of the user's groups (incl. Everyone).

use serde::Serialize;
use uuid::Uuid;

use crate::{error::AppResult, providers::{MODEL_COLS, Model}, state::AppState};

#[derive(Serialize, sqlx::FromRow)]
pub struct ModelAccess {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub model: Model,
    pub is_public: bool,
    pub direct: bool,
    pub via_groups: Vec<String>,
}

impl ModelAccess {
    pub fn allowed(&self) -> bool {
        self.is_public || self.direct || !self.via_groups.is_empty()
    }
}

// ponytail: evaluated with one indexed query per request instead of the in-memory per-user cache the spec
// describes; add a moka cache invalidated on grant/group/model changes if this shows up in latency profiles.
pub async fn models_for_user(state: &AppState, user_id: Uuid, include_disabled: bool) -> AppResult<Vec<ModelAccess>> {
    let rows: Vec<ModelAccess> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {MODEL_COLS},
            m.visibility = 'public' as is_public,
            exists(select 1 from grants g where g.model_id = m.id and g.user_id = $1) as direct,
            array(select gr.name from grants g join groups gr on gr.id = g.group_id
                  where g.model_id = m.id and (gr.is_everyone or gr.id in (select group_id from group_members where user_id = $1))
                  order by gr.name) as via_groups
         from models m join providers p on p.id = m.provider_id
         where $2 or (m.enabled and p.enabled)
         order by m.display_name"
    )))
    .bind(user_id)
    .bind(include_disabled)
    .fetch_all(&state.db)
    .await?;
    Ok(rows)
}

pub async fn accessible_models(state: &AppState, user_id: Uuid) -> AppResult<Vec<Model>> {
    Ok(models_for_user(state, user_id, false).await?.into_iter().filter(|m| m.allowed()).map(|m| m.model).collect())
}

pub enum ModelRef<'a> {
    Id(Uuid),
    Name(&'a str),
}

pub struct Resolved {
    pub model: Model,
    pub provider: crate::upstream::Provider,
}

/// Access check in the spec's order: model exists and is enabled (404), provider has a usable key (503),
/// then public / direct grant / group grant (403). The caller has already verified the user is active.
pub async fn resolve(state: &AppState, user_id: Uuid, r: ModelRef<'_>) -> AppResult<Resolved> {
    use crate::error::AppError;
    let model: Option<Model> = match r {
        ModelRef::Id(id) => {
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "select {MODEL_COLS} from models m join providers p on p.id = m.provider_id where m.id = $1 and m.enabled and p.enabled"
            )))
            .bind(id)
            .fetch_optional(&state.db)
            .await?
        }
        ModelRef::Name(name) => {
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "select {MODEL_COLS} from models m join providers p on p.id = m.provider_id where m.name = $1 and m.enabled and p.enabled"
            )))
            .bind(name)
            .fetch_optional(&state.db)
            .await?
        }
    };
    let model = model.ok_or_else(|| AppError::new(axum::http::StatusCode::NOT_FOUND, "model_not_found", "This model does not exist or is disabled"))?;
    let (keys, healthy): (i64, i64) = sqlx::query_as(
        "select count(*), count(*) filter (where healthy) from provider_keys where provider_id = $1 and enabled",
    )
    .bind(model.provider_id)
    .fetch_one(&state.db)
    .await?;
    if keys > 0 && healthy == 0 {
        return Err(AppError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "provider_unavailable",
            "This model's provider has no working API key. Ask an admin to check the provider.",
        ));
    }
    let allowed: bool = model.visibility == "public"
        || sqlx::query_scalar(
            "select exists(select 1 from grants g where g.model_id = $1 and (g.user_id = $2 or g.group_id in (
                select group_id from group_members where user_id = $2 union select id from groups where is_everyone)))",
        )
        .bind(model.id)
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    if !allowed {
        return Err(AppError::new(axum::http::StatusCode::FORBIDDEN, "model_forbidden", "You don't have access to this model"));
    }
    let provider = crate::upstream::load_provider(state, model.provider_id).await?;
    Ok(Resolved { model, provider })
}

/// User override, else any group (including Everyone) that allows it, else the global setting.
pub async fn can_use_api_keys(state: &AppState, user_id: Uuid) -> AppResult<bool> {
    let global = crate::settings::get(state).await?.allow_api_keys;
    Ok(sqlx::query_scalar(
        "select coalesce(u.allow_api_keys, (select bool_or(g.allow_api_keys) from groups g
            left join group_members m on m.group_id = g.id and m.user_id = u.id
            where g.is_everyone or m.user_id is not null), $2)
         from users u where u.id = $1",
    )
    .bind(user_id)
    .bind(global)
    .fetch_one(&state.db)
    .await?)
}
