//! Saved import layout templates: named regex + role + value-map presets that
//! carve a staged import into models/variants (services/layout.rs). Shipped
//! presets are seeded by migration; users save their own from the import page.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::services::layout::{CarveTarget, LayoutSpec, analyze, canonical_value_map};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/import-layouts", get(list).post(create))
        .route(
            "/api/import-layouts/{id}",
            axum::routing::put(update).delete(remove),
        )
}

#[derive(Serialize, ToSchema)]
pub struct ImportLayout {
    pub id: Uuid,
    pub name: String,
    #[serde(flatten)]
    pub spec: LayoutSpec,
    /// The publisher whose archives this layout fits (auto-suggestion).
    pub creator_id: Option<Uuid>,
}

#[derive(Deserialize, ToSchema)]
pub struct LayoutInput {
    pub name: String,
    #[serde(flatten)]
    pub spec: LayoutSpec,
    pub creator_id: Option<Uuid>,
}

/// A saved layout must at least compile and name real capture groups; running
/// [`analyze`] over no files checks exactly that, with the engine that will
/// interpret it later.
fn validate(input: &LayoutInput) -> Result<String, ApiError> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    analyze(&input.spec, CarveTarget::Bundle, &[], &Default::default())?;
    Ok(name)
}

fn name_conflict(e: sqlx::Error) -> ApiError {
    match e {
        sqlx::Error::Database(ref db) if db.is_unique_violation() => {
            ApiError::Conflict("a layout with that name already exists".into())
        }
        e => e.into(),
    }
}

type SpecJson = sqlx::types::Json<LayoutSpec>;

async fn list(
    State(state): State<AppState>,
    _user: User,
) -> Result<Json<Vec<ImportLayout>>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT id, name::text as "name!", creator_id,
                  jsonb_build_object('pattern', pattern, 'roles', roles,
                                     'value_map', value_map, 'flatten', flatten)
                      as "spec!: SpecJson"
           FROM import_layouts ORDER BY name"#,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ImportLayout {
                id: r.id,
                name: r.name,
                spec: r.spec.0,
                creator_id: r.creator_id,
            })
            .collect(),
    ))
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(mut input): Json<LayoutInput>,
) -> Result<Json<ImportLayout>, ApiError> {
    user.require_editor()?;
    let name = validate(&input)?;
    // Store canonical value-map keys, so a template never carries two spellings
    // of one value that a carve then resolves by hash order (see layout.rs).
    input.spec.value_map = canonical_value_map(&input.spec.value_map)
        .into_iter()
        .collect();
    let id: Uuid = sqlx::query_scalar!(
        "INSERT INTO import_layouts (name, pattern, roles, value_map, flatten, creator_id, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
        name,
        input.spec.pattern,
        serde_json::to_value(&input.spec.roles).expect("roles serialize"),
        serde_json::to_value(&input.spec.value_map).expect("value_map serialize"),
        input.spec.flatten,
        input.creator_id,
        user.id,
    )
    .fetch_one(&state.db)
    .await
    .map_err(name_conflict)?;
    Ok(Json(ImportLayout {
        id,
        name,
        spec: input.spec,
        creator_id: input.creator_id,
    }))
}

async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(mut input): Json<LayoutInput>,
) -> Result<Json<ImportLayout>, ApiError> {
    user.require_editor()?;
    let name = validate(&input)?;
    input.spec.value_map = canonical_value_map(&input.spec.value_map)
        .into_iter()
        .collect();
    let updated = sqlx::query!(
        "UPDATE import_layouts
            SET name = $2, pattern = $3, roles = $4, value_map = $5, flatten = $6,
                creator_id = $7, updated_at = now()
          WHERE id = $1",
        id,
        name,
        input.spec.pattern,
        serde_json::to_value(&input.spec.roles).expect("roles serialize"),
        serde_json::to_value(&input.spec.value_map).expect("value_map serialize"),
        input.spec.flatten,
        input.creator_id,
    )
    .execute(&state.db)
    .await
    .map_err(name_conflict)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(Json(ImportLayout {
        id,
        name,
        spec: input.spec,
        creator_id: input.creator_id,
    }))
}

async fn remove(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    user.require_editor()?;
    sqlx::query!("DELETE FROM import_layouts WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
