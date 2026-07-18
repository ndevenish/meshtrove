//! Free-form tags for models and bundles.

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::models::{
    parse_csv, push_model_tag_filters, push_text_filter, push_variant_group,
};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/tags", get(list).post(create))
}

#[derive(Serialize, ToSchema)]
pub struct Tag {
    pub id: Uuid,
    pub name: String,
    pub model_count: i64,
}

#[derive(Deserialize)]
pub struct ListQuery {
    /// Name substring, for autocomplete pickers.
    pub q: Option<String>,
    /// The browse page's current selection. When present, each tag's
    /// `model_count` becomes a co-occurrence count: how many models match the
    /// selection *and* also carry this tag — so adding tags filters the numbers
    /// down. With no selection this reduces to the plain global count.
    pub sel_tags: Option<String>,
    pub sel_vtags: Option<String>,
    pub sel_q: Option<String>,
}

async fn list(
    State(state): State<AppState>,
    _user: User,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<Tag>>, ApiError> {
    let name = query.q.unwrap_or_default();
    let sel_tags = parse_csv(&query.sel_tags.unwrap_or_default());
    let sel_vtags = parse_csv(&query.sel_vtags.unwrap_or_default());
    let sel_q = query.sel_q.unwrap_or_default().trim().to_string();

    // model_count = models matching the current selection that also carry this
    // tag `t`. The candidate clause correlates to the outer `t`; the selection's
    // own filters use alias `ft` internally, so they don't shadow it.
    let mut qb = QueryBuilder::new(
        "SELECT t.id, t.name::text AS name, (SELECT count(*) FROM models m WHERE TRUE",
    );
    push_text_filter(&mut qb, &sel_q);
    push_model_tag_filters(&mut qb, &sel_tags);
    push_variant_group(&mut qb, &sel_vtags);
    qb.push(
        " AND EXISTS (SELECT 1 FROM model_tags mt WHERE mt.model_id = m.id AND mt.tag_id = t.id)) \
         AS model_count FROM tags t WHERE (",
    );
    qb.push_bind(name.clone())
        .push(" = '' OR t.name ILIKE '%' || ")
        .push_bind(name.clone())
        .push(" || '%') ORDER BY model_count DESC, t.name");

    let rows = qb.build().fetch_all(&state.db).await?;
    let tags = rows
        .into_iter()
        .map(|r| -> Result<Tag, sqlx::Error> {
            Ok(Tag {
                id: r.try_get("id")?,
                name: r.try_get("name")?,
                model_count: r.try_get("model_count")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(tags))
}

#[derive(Deserialize, ToSchema)]
pub struct TagInput {
    pub name: String,
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<TagInput>,
) -> Result<Json<Tag>, ApiError> {
    user.require_editor()?;
    let tag = upsert_tag(&state.db, &input.name).await?;
    Ok(Json(tag))
}

/// Get-or-create by (case-insensitive) name; reused by model tagging.
///
/// Runs on whatever executor the caller passes — a pool for a one-off, or the
/// caller's own `&mut *tx` when it is mid-transaction. Callers that hold an open
/// write transaction MUST pass that transaction: upserting on a second pooled
/// connection while the first holds `model_tags`/`bundle_tags` FK locks made one
/// logical write straddle two connections, and a large patch (dozens of shared
/// tags) could stall for tens of seconds waiting on locks its own transaction
/// held.
pub async fn upsert_tag<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    name: &str,
) -> Result<Tag, ApiError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("tag name is required".into()));
    }
    // Insert-or-get in one statement. `DO NOTHING` + a `SELECT` fallback used to
    // race: two concurrent inserts of the same new tag both come back empty — the
    // loser's INSERT skips (no RETURNING row) and its SELECT runs under a snapshot
    // taken before the winner committed, so it sees nothing either. `fetch_one`
    // then hit `RowNotFound`, which the error layer turns into a spurious 404.
    // `DO UPDATE SET name = tags.name` is a no-op self-update that still locks and
    // RETURNs the existing row (keeping its original casing — `name` is citext),
    // so we always get exactly one row back.
    let tag = sqlx::query!(
        r#"INSERT INTO tags (name) VALUES ($1)
           ON CONFLICT (name) DO UPDATE SET name = tags.name
           RETURNING id, name as "name!: String""#,
        name,
    )
    .fetch_one(executor)
    .await?;
    Ok(Tag {
        id: tag.id,
        name: tag.name,
        model_count: 0,
    })
}
