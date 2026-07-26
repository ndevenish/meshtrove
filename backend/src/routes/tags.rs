//! Free-form tags for models and bundles.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::models::{
    parse_csv, push_model_hidden_exclude, push_model_tag_filters, push_text_filter,
    push_variant_group,
};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/tags", get(list).post(create))
        // Admin: every tag (hidden ones included) with its global usage count,
        // and the toggle that hides/unhides one from browsing and search.
        .route("/api/tags/manage", get(list_all))
        .route("/api/tags/{id}/hidden", put(set_hidden))
}

#[derive(Serialize, ToSchema)]
pub struct Tag {
    pub id: Uuid,
    pub name: String,
    pub model_count: i64,
    /// Suppressed from the filter sidebar, model/bundle cards and detail pages,
    /// and the search index. The public `list` never returns hidden tags, so
    /// this is only ever `true` in the admin `list_all` view.
    pub hidden: bool,
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
    /// Admin-only: include hidden tags (and count hidden items), paired with the
    /// browse "Show hidden" toggle. ANDed with the caller being an admin.
    pub show_hidden: Option<bool>,
}

async fn list(
    State(state): State<AppState>,
    user: User,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<Tag>>, ApiError> {
    let name = query.q.unwrap_or_default();
    let sel_tags = parse_csv(&query.sel_tags.unwrap_or_default());
    let sel_vtags = parse_csv(&query.sel_vtags.unwrap_or_default());
    let sel_q = query.sel_q.unwrap_or_default().trim().to_string();
    let show_hidden = query.show_hidden.unwrap_or(false) && user.is_admin();

    // model_count = *visible* models matching the current selection that also
    // carry this tag `t`. Joining tags → model_tags → models and grouping counts
    // every tag in one aggregate pass; the INNER joins mean a tag with no
    // matching model produces no row, so `count > 0` is automatic — that alone
    // drops hidden tags (their models are all hidden, so nothing survives
    // `NOT m.hidden`), orphan tags, and, while filtering, any tag no selected
    // model carries. The selection's own filters use alias `ft`/`mt` internally,
    // so the join alias is `l` to avoid shadowing them.
    let mut qb = QueryBuilder::new(
        "SELECT t.id, t.name::text AS name, t.hidden, count(*) AS model_count \
         FROM tags t JOIN model_tags l ON l.tag_id = t.id \
         JOIN models m ON m.id = l.model_id WHERE (",
    );
    qb.push_bind(name.clone())
        .push(" = '' OR t.name ILIKE '%' || ")
        .push_bind(name.clone())
        .push(" || '%')");
    push_text_filter(&mut qb, &sel_q);
    push_model_tag_filters(&mut qb, &sel_tags);
    push_variant_group(&mut qb, &sel_vtags);
    push_model_hidden_exclude(&mut qb, show_hidden);
    qb.push(" GROUP BY t.id, t.name, t.hidden ORDER BY model_count DESC, t.name");

    let rows = qb.build().fetch_all(&state.db).await?;
    let tags = rows
        .into_iter()
        .map(|r| -> Result<Tag, sqlx::Error> {
            Ok(Tag {
                id: r.try_get("id")?,
                name: r.try_get("name")?,
                model_count: r.try_get("model_count")?,
                hidden: r.try_get("hidden")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(tags))
}

/// Admin: the whole vocabulary, hidden tags included, each with its global model
/// usage count. Powers the admin Tags panel; hidden tags sort last.
async fn list_all(State(state): State<AppState>, user: User) -> Result<Json<Vec<Tag>>, ApiError> {
    user.require_admin()?;
    let rows = sqlx::query!(
        r#"SELECT t.id, t.name::text AS "name!", t.hidden,
                  (SELECT count(*) FROM model_tags mt WHERE mt.tag_id = t.id) AS "model_count!"
             FROM tags t
            ORDER BY t.hidden,
                     (SELECT count(*) FROM model_tags mt WHERE mt.tag_id = t.id) DESC,
                     t.name"#,
    )
    .fetch_all(&state.db)
    .await?;
    let tags = rows
        .into_iter()
        .map(|r| Tag {
            id: r.id,
            name: r.name,
            model_count: r.model_count,
            hidden: r.hidden,
        })
        .collect();
    Ok(Json(tags))
}

#[derive(Deserialize, ToSchema)]
pub struct HiddenInput {
    pub hidden: bool,
}

/// Admin: hide or unhide a tag. The DB triggers re-index every model and bundle
/// carrying it, so the change reaches search immediately; the tag's association
/// rows are left untouched, so unhiding restores it exactly as it was.
async fn set_hidden(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<HiddenInput>,
) -> Result<Json<Tag>, ApiError> {
    user.require_admin()?;
    let row = sqlx::query!(
        r#"UPDATE tags SET hidden = $2 WHERE id = $1
           RETURNING id, name::text AS "name!", hidden,
                     (SELECT count(*) FROM model_tags mt WHERE mt.tag_id = $1) AS "model_count!""#,
        id,
        input.hidden,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(Tag {
        id: row.id,
        name: row.name,
        model_count: row.model_count,
        hidden: row.hidden,
    }))
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
        hidden: false,
    })
}
