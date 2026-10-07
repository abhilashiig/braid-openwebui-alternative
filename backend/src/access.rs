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
