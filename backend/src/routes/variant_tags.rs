//! The flat variant-tag vocabulary ("32mm", "supported", "lychee"). Kept
//! separate from `tags`, which describes what a model *is* rather than which
//! edition of it a file belongs to. Tags are get-or-created inline when a
//! variant is tagged, so the vocabulary grows during import; these routes exist
//! to list it for autocomplete and to let an editor tidy it up afterwards.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::models::{
    parse_csv, push_model_tag_filters, push_text_filter, push_variant_tag_filters,
};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/variant-tags", get(list).post(create))
        .route(
            "/api/variant-tags/{id}",
            axum::routing::put(update).delete(remove),
        )
}

#[derive(Serialize, ToSchema)]
pub struct VariantTag {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub variant_count: i64,
}

#[derive(Deserialize)]
pub struct ListQuery {
    /// Name substring, for autocomplete pickers.
    pub q: Option<String>,
    /// The browse page's current selection. When present, `variant_count`
    /// switches from a global per-variant tally to a co-occurrence count: how
    /// many *models* would still match if this variant tag were added to the
    /// selection (one variant carrying the whole set, as the browse filter
    /// requires). With no selection the plain per-variant count is returned, so
    /// autocomplete callers are unaffected.
    pub sel_tags: Option<String>,
    pub sel_vtags: Option<String>,
    pub sel_q: Option<String>,
}

async fn list(
    State(state): State<AppState>,
    _user: User,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<VariantTag>>, ApiError> {
    let name = query.q.unwrap_or_default();
    let sel_tags = parse_csv(&query.sel_tags.unwrap_or_default());
    let sel_vtags = parse_csv(&query.sel_vtags.unwrap_or_default());
    let sel_q = query.sel_q.unwrap_or_default().trim().to_string();
    let has_selection = !sel_tags.is_empty() || !sel_vtags.is_empty() || !sel_q.is_empty();

    let mut qb = QueryBuilder::new("SELECT t.id, t.name::text AS name, t.description, ");
    if has_selection {
        // Count models that would survive adding this candidate `t`: one variant
        // must carry the candidate *and* every already-selected variant tag.
        qb.push("(SELECT count(*) FROM models m WHERE TRUE");
        push_text_filter(&mut qb, &sel_q);
        push_model_tag_filters(&mut qb, &sel_tags);
        qb.push(
            " AND EXISTS (SELECT 1 FROM model_variants v WHERE v.model_id = m.id \
             AND EXISTS (SELECT 1 FROM variant_tag_assignments ca \
             WHERE ca.variant_id = v.id AND ca.tag_id = t.id)",
        );
        push_variant_tag_filters(&mut qb, &sel_vtags);
        qb.push(")) AS variant_count ");
    } else {
        qb.push(
            "(SELECT count(*) FROM variant_tag_assignments a WHERE a.tag_id = t.id) \
             AS variant_count ",
        );
    }
    qb.push("FROM variant_tags t WHERE (")
        .push_bind(name.clone())
        .push(" = '' OR t.name ILIKE '%' || ")
        .push_bind(name.clone())
        .push(" || '%') ORDER BY variant_count DESC, t.name");

    let rows = qb.build().fetch_all(&state.db).await?;
    let tags = rows
        .into_iter()
        .map(|r| -> Result<VariantTag, sqlx::Error> {
            Ok(VariantTag {
                id: r.try_get("id")?,
                name: r.try_get("name")?,
                description: r.try_get("description")?,
                variant_count: r.try_get("variant_count")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(tags))
}

#[derive(Deserialize, ToSchema)]
pub struct VariantTagInput {
    pub name: String,
    pub description: Option<String>,
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<VariantTagInput>,
) -> Result<Json<VariantTag>, ApiError> {
    user.require_editor()?;
    let id = upsert_variant_tag(&state.db, &input.name).await?;
    if let Some(description) = input.description.as_deref() {
        sqlx::query!(
            "UPDATE variant_tags SET description = $2 WHERE id = $1",
            id,
            description,
        )
        .execute(&state.db)
        .await?;
    }
    Ok(Json(VariantTag {
        id,
        name: input.name.trim().to_string(),
        description: input.description,
        variant_count: 0,
    }))
}

/// Get-or-create by (case-insensitive) name. Takes a connection rather than
/// `AppState` so variant tagging can run it inside its transaction.
pub async fn upsert_variant_tag(
    db: impl sqlx::PgExecutor<'_>,
    name: &str,
) -> Result<Uuid, ApiError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("variant tag name is required".into()));
    }
    let id = sqlx::query_scalar!(
        r#"WITH ins AS (
               INSERT INTO variant_tags (name) VALUES ($1)
               ON CONFLICT (name) DO NOTHING RETURNING id
           )
           SELECT id as "id!" FROM ins
           UNION ALL SELECT id FROM variant_tags WHERE name = $1
           LIMIT 1"#,
        name,
    )
    .fetch_one(db)
    .await?;
    Ok(id)
}

async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<VariantTagInput>,
) -> Result<Json<VariantTag>, ApiError> {
    user.require_editor()?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("variant tag name is required".into()));
    }
    // Renaming cannot change any variant's identity: `tag_key` is built from tag
    // ids, not names, so no merge check is needed here.
    let row = sqlx::query!(
        r#"UPDATE variant_tags SET name = $2, description = $3 WHERE id = $1
           RETURNING id, name as "name!: String", description"#,
        id,
        name,
        input.description,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(e) if e.is_unique_violation() => {
            ApiError::Conflict(format!("variant tag {name:?} already exists"))
        }
        e => e.into(),
    })?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(VariantTag {
        id: row.id,
        name: row.name,
        description: row.description,
        variant_count: 0,
    }))
}

/// Deleting a tag drops it from every variant carrying it — which can leave two
/// variants of one model with the same remaining tag set. Same tag set is the
/// same variant, so they merge rather than colliding.
async fn remove(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    user.require_editor()?;
    let mut tx = state.db.begin().await?;
    sqlx::query!("DELETE FROM variant_tags WHERE id = $1", id)
        .execute(&mut *tx)
        .await?;
    sqlx::query!("SELECT merge_duplicate_variants()")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
