//! Models: CRUD, unified search (text + tags + variant axis options), and
//! markdown description revisions.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, put},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::tags::upsert_tag;
use crate::routes::variants::{VariantDetail, fetch_variants};
use crate::state::AppState;
use crate::util::slugify;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/models", get(search).post(create))
        .route("/api/models/{id}", get(detail).put(update).delete(remove))
        .route("/api/models/{id}/description", put(update_description))
        .route(
            "/api/models/{id}/description/revisions",
            get(list_revisions),
        )
        .route(
            "/api/models/{id}/description/revisions/{rev}/label",
            put(label_revision),
        )
}

// ---------------------------------------------------------------------------
// search
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SearchQuery {
    /// Full-text + fuzzy name query
    pub q: Option<String>,
    /// Comma-separated tag names, all required
    pub tags: Option<String>,
    /// Comma-separated `axis:value` pairs; a single variant must satisfy all
    pub opts: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Serialize, ToSchema)]
pub struct ModelSummary {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub creator_id: Option<Uuid>,
    pub creator_name: Option<String>,
    pub primary_image_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub like_count: i64,
    pub variant_count: i64,
    /// Variants satisfying the `opts` filter (when one was given)
    pub matched_variant_ids: Option<Vec<Uuid>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
pub struct SearchResults {
    pub models: Vec<ModelSummary>,
    pub total: i64,
    pub page: u32,
    pub per_page: u32,
}

pub fn parse_opts(opts: &str) -> Result<Vec<(String, String)>, ApiError> {
    opts.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|pair| {
            pair.split_once(':')
                .map(|(a, v)| (a.trim().to_string(), v.trim().to_string()))
                .ok_or_else(|| {
                    ApiError::BadRequest(format!("opts entry {pair:?} is not axis:value"))
                })
        })
        .collect()
}

/// Append the shared WHERE clauses for a search to a query builder (alias `m`).
pub fn push_filters(
    qb: &mut QueryBuilder<sqlx::Postgres>,
    q: &str,
    tags: &[String],
    opts: &[(String, String)],
) {
    if !q.is_empty() {
        qb.push(" AND (m.search @@ websearch_to_tsquery('english', ")
            .push_bind(q.to_string())
            .push(") OR m.name ILIKE '%' || ")
            .push_bind(q.to_string())
            .push(" || '%')");
    }
    for tag in tags {
        qb.push(" AND EXISTS (SELECT 1 FROM model_tags mt JOIN tags t ON t.id = mt.tag_id WHERE mt.model_id = m.id AND t.name = ")
            .push_bind(tag.clone())
            .push(")");
    }
    if !opts.is_empty() {
        // One variant must satisfy ALL requested axis:value pairs at once.
        qb.push(" AND EXISTS (SELECT 1 FROM model_variants v WHERE v.model_id = m.id");
        for (axis, value) in opts {
            qb.push(" AND EXISTS (SELECT 1 FROM variant_options vo JOIN variant_axes a ON a.id = vo.axis_id JOIN variant_axis_options o ON o.axis_id = vo.axis_id AND o.id = vo.option_id WHERE vo.variant_id = v.id AND a.name = ")
                .push_bind(axis.clone())
                .push(" AND o.value = ")
                .push_bind(value.clone())
                .push(")");
        }
        qb.push(")");
    }
}

async fn search(
    State(state): State<AppState>,
    _user: User,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResults>, ApiError> {
    let q = query.q.unwrap_or_default().trim().to_string();
    let tags: Vec<String> = query
        .tags
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let opts = parse_opts(&query.opts.unwrap_or_default())?;
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(30).clamp(1, 100);

    let mut count_qb = QueryBuilder::new("SELECT count(*) FROM models m WHERE TRUE");
    push_filters(&mut count_qb, &q, &tags, &opts);
    let total: i64 = count_qb.build_query_scalar().fetch_one(&state.db).await?;

    let mut qb = QueryBuilder::new(
        r#"SELECT m.id, m.name, m.slug, m.creator_id, m.updated_at, c.name AS creator_name,
              (SELECT i.id FROM images i WHERE i.model_id = m.id AND i.is_primary) AS primary_image_id,
              (SELECT count(*) FROM user_model_marks k WHERE k.model_id = m.id AND k.mark = 'liked') AS like_count,
              (SELECT count(*) FROM model_variants v WHERE v.model_id = m.id) AS variant_count,
              coalesce((SELECT array_agg(t.name::text ORDER BY t.name) FROM model_tags mt
                        JOIN tags t ON t.id = mt.tag_id WHERE mt.model_id = m.id), '{}') AS tags
         FROM models m LEFT JOIN creators c ON c.id = m.creator_id WHERE TRUE"#,
    );
    push_filters(&mut qb, &q, &tags, &opts);
    if q.is_empty() {
        qb.push(" ORDER BY m.updated_at DESC");
    } else {
        qb.push(" ORDER BY ts_rank(m.search, websearch_to_tsquery('english', ")
            .push_bind(&q)
            .push(")) DESC, m.updated_at DESC");
    }
    qb.push(" LIMIT ")
        .push_bind(per_page as i64)
        .push(" OFFSET ")
        .push_bind(((page - 1) * per_page) as i64);

    let rows: Vec<sqlx::postgres::PgRow> = qb.build().fetch_all(&state.db).await?;
    let mut models: Vec<ModelSummary> = rows
        .into_iter()
        .map(
            |row: sqlx::postgres::PgRow| -> Result<ModelSummary, sqlx::Error> {
                Ok(ModelSummary {
                    id: row.try_get("id")?,
                    name: row.try_get("name")?,
                    slug: row.try_get("slug")?,
                    creator_id: row.try_get("creator_id")?,
                    creator_name: row.try_get("creator_name")?,
                    primary_image_id: row.try_get("primary_image_id")?,
                    like_count: row.try_get("like_count")?,
                    variant_count: row.try_get("variant_count")?,
                    tags: row.try_get("tags")?,
                    matched_variant_ids: None,
                    updated_at: row.try_get("updated_at")?,
                })
            },
        )
        .collect::<Result<_, _>>()
        .context("decoding search row")?;

    // Mark which variants matched the opts filter so the UI can highlight them.
    if !opts.is_empty() && !models.is_empty() {
        let model_ids: Vec<Uuid> = models.iter().map(|m| m.id).collect();
        let mut vq = QueryBuilder::new(
            "SELECT v.id, v.model_id FROM model_variants v WHERE v.model_id = ANY(",
        );
        vq.push_bind(model_ids).push(")");
        for (axis, value) in &opts {
            vq.push(" AND EXISTS (SELECT 1 FROM variant_options vo JOIN variant_axes a ON a.id = vo.axis_id JOIN variant_axis_options o ON o.axis_id = vo.axis_id AND o.id = vo.option_id WHERE vo.variant_id = v.id AND a.name = ")
                .push_bind(axis.clone())
                .push(" AND o.value = ")
                .push_bind(value.clone())
                .push(")");
        }
        let variant_rows: Vec<sqlx::postgres::PgRow> = vq.build().fetch_all(&state.db).await?;
        for model in &mut models {
            let matched: Vec<Uuid> = variant_rows
                .iter()
                .filter(|r| r.get::<Uuid, _>("model_id") == model.id)
                .map(|r| r.get::<Uuid, _>("id"))
                .collect();
            model.matched_variant_ids = Some(matched);
        }
    }

    Ok(Json(SearchResults {
        models,
        total,
        page,
        per_page,
    }))
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct ModelInput {
    pub name: String,
    pub creator_id: Option<Uuid>,
    pub source_url: Option<String>,
    pub license: Option<String>,
    pub purchase_price: Option<f64>,
    pub purchase_date: Option<NaiveDate>,
    pub order_ref: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Initial markdown description (creates revision 1)
    pub description_md: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ModelDetail {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub creator_id: Option<Uuid>,
    pub creator_name: Option<String>,
    pub source_url: Option<String>,
    pub license: Option<String>,
    pub purchase_price: Option<f64>,
    pub purchase_date: Option<NaiveDate>,
    pub order_ref: Option<String>,
    pub tags: Vec<String>,
    pub description_md: Option<String>,
    pub variants: Vec<VariantDetail>,
    pub images: Vec<ImageSummary>,
    /// Bundles this model is a member of (so the UI can link, and avoid
    /// promoting the same model into a duplicate bundle).
    pub bundles: Vec<BundleRef>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
pub struct BundleRef {
    pub id: Uuid,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct ImageSummary {
    pub id: Uuid,
    pub kind: String,
    pub is_primary: bool,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

async fn unique_slug(state: &AppState, name: &str) -> Result<String, ApiError> {
    let base = slugify(name);
    let taken: Vec<String> = sqlx::query_scalar!(
        "SELECT slug FROM models WHERE slug = $1 OR slug LIKE $1 || '-%'",
        base,
    )
    .fetch_all(&state.db)
    .await?;
    if !taken.iter().any(|s| s == &base) {
        return Ok(base);
    }
    for n in 2.. {
        let candidate = format!("{base}-{n}");
        if !taken.iter().any(|s| s == &candidate) {
            return Ok(candidate);
        }
    }
    unreachable!()
}

async fn set_model_tags(
    tx: &mut sqlx::PgConnection,
    state: &AppState,
    model_id: Uuid,
    tags: &[String],
) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM model_tags WHERE model_id = $1", model_id)
        .execute(&mut *tx)
        .await?;
    for tag in tags {
        let tag = upsert_tag(state, tag).await?;
        sqlx::query!(
            "INSERT INTO model_tags (model_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            model_id,
            tag.id,
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<ModelInput>,
) -> Result<Json<ModelDetail>, ApiError> {
    user.require_editor()?;
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    let slug = unique_slug(&state, &name).await?;

    let mut tx = state.db.begin().await?;
    let model_id: Uuid = sqlx::query_scalar!(
        r#"INSERT INTO models (name, slug, creator_id, source_url, license,
                               purchase_price, purchase_date, order_ref, created_by)
           VALUES ($1, $2, $3, $4, $5, $6::float8::numeric(10,2), $7, $8, $9)
           RETURNING id"#,
        name,
        slug,
        input.creator_id,
        input.source_url,
        input.license,
        input.purchase_price,
        input.purchase_date,
        input.order_ref,
        user.id,
    )
    .fetch_one(&mut *tx)
    .await?;

    set_model_tags(&mut tx, &state, model_id, &input.tags).await?;

    if let Some(body) = &input.description_md {
        sqlx::query!(
            "INSERT INTO model_description_revisions (model_id, body_md, created_by)
             VALUES ($1, $2, $3)",
            model_id,
            body,
            user.id,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    fetch_detail(&state, model_id).await.map(Json)
}

async fn fetch_detail(state: &AppState, id: Uuid) -> Result<ModelDetail, ApiError> {
    let row = sqlx::query!(
        r#"SELECT m.id, m.name, m.slug, m.creator_id, c.name as "creator_name?",
                  m.source_url, m.license, m.purchase_price::float8 as purchase_price,
                  m.purchase_date, m.order_ref, m.created_by, m.created_at, m.updated_at,
                  (SELECT r.body_md FROM model_description_revisions r
                    WHERE r.model_id = m.id ORDER BY r.created_at DESC LIMIT 1) as description_md,
                  coalesce((SELECT array_agg(t.name::text ORDER BY t.name) FROM model_tags mt
                            JOIN tags t ON t.id = mt.tag_id WHERE mt.model_id = m.id), '{}') as "tags!"
           FROM models m LEFT JOIN creators c ON c.id = m.creator_id
           WHERE m.id = $1"#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let images = sqlx::query!(
        r#"SELECT id, kind::text as "kind!", is_primary, width, height FROM images
           WHERE model_id = $1 ORDER BY is_primary DESC, sort_order, created_at"#,
        id,
    )
    .fetch_all(&state.db)
    .await?;

    let variants = fetch_variants(state, id).await?;

    let bundles = sqlx::query_as!(
        BundleRef,
        r#"SELECT b.id, b.name FROM bundles b
           JOIN bundle_models bm ON bm.bundle_id = b.id
           WHERE bm.model_id = $1 ORDER BY b.name"#,
        id,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(ModelDetail {
        id: row.id,
        name: row.name,
        slug: row.slug,
        creator_id: row.creator_id,
        creator_name: row.creator_name,
        source_url: row.source_url,
        license: row.license,
        purchase_price: row.purchase_price,
        purchase_date: row.purchase_date,
        order_ref: row.order_ref,
        tags: row.tags,
        description_md: row.description_md,
        variants,
        bundles,
        images: images
            .into_iter()
            .map(|i| ImageSummary {
                id: i.id,
                kind: i.kind,
                is_primary: i.is_primary,
                width: i.width,
                height: i.height,
            })
            .collect(),
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

async fn detail(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<ModelDetail>, ApiError> {
    fetch_detail(&state, id).await.map(Json)
}

pub async fn model_created_by(state: &AppState, id: Uuid) -> Result<Uuid, ApiError> {
    sqlx::query_scalar!("SELECT created_by FROM models WHERE id = $1", id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)
}

async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<ModelInput>,
) -> Result<Json<ModelDetail>, ApiError> {
    user.require_can_edit(model_created_by(&state, id).await?)?;

    let mut tx = state.db.begin().await?;
    sqlx::query!(
        r#"UPDATE models SET name = $2, creator_id = $3, source_url = $4, license = $5,
               purchase_price = $6::float8::numeric(10,2), purchase_date = $7,
               order_ref = $8, updated_at = now()
           WHERE id = $1"#,
        id,
        input.name.trim(),
        input.creator_id,
        input.source_url,
        input.license,
        input.purchase_price,
        input.purchase_date,
        input.order_ref,
    )
    .execute(&mut *tx)
    .await?;
    set_model_tags(&mut tx, &state, id, &input.tags).await?;
    tx.commit().await?;

    fetch_detail(&state, id).await.map(Json)
}

async fn remove(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    user.require_can_edit(model_created_by(&state, id).await?)?;
    sqlx::query!("DELETE FROM models WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    // Blobs are left in place: content-addressed and possibly shared. A future
    // GC job sweeps orphans (docs/plan.md).
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// description revisions
// ---------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct DescriptionInput {
    pub body_md: String,
}

#[derive(Serialize, ToSchema)]
pub struct Revision {
    pub id: Uuid,
    pub body_md: String,
    pub label: Option<String>,
    pub created_by: Uuid,
    pub author: String,
    pub created_at: DateTime<Utc>,
}

async fn update_description(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<DescriptionInput>,
) -> Result<Json<Revision>, ApiError> {
    user.require_can_edit(model_created_by(&state, id).await?)?;
    let row = sqlx::query!(
        r#"INSERT INTO model_description_revisions (model_id, body_md, created_by)
           VALUES ($1, $2, $3)
           RETURNING id, body_md, label as "label: String", created_by, created_at"#,
        id,
        input.body_md,
        user.id,
    )
    .fetch_one(&state.db)
    .await?;
    // updated_at drives default search ordering
    sqlx::query!("UPDATE models SET updated_at = now() WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    Ok(Json(Revision {
        id: row.id,
        body_md: row.body_md,
        label: row.label,
        created_by: row.created_by,
        author: user.username,
        created_at: row.created_at,
    }))
}

async fn list_revisions(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<Revision>>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT r.id, r.body_md, r.label as "label: String", r.created_by, r.created_at,
                  u.username as "author: String"
           FROM model_description_revisions r JOIN users u ON u.id = r.created_by
           WHERE r.model_id = $1 ORDER BY r.created_at DESC"#,
        id,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| Revision {
                id: r.id,
                body_md: r.body_md,
                label: r.label,
                created_by: r.created_by,
                author: r.author,
                created_at: r.created_at,
            })
            .collect(),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct LabelInput {
    /// e.g. "v1"; null clears the label
    pub label: Option<String>,
}

async fn label_revision(
    State(state): State<AppState>,
    user: User,
    Path((id, rev)): Path<(Uuid, Uuid)>,
    Json(input): Json<LabelInput>,
) -> Result<StatusCode, ApiError> {
    user.require_can_edit(model_created_by(&state, id).await?)?;
    let result = sqlx::query!(
        "UPDATE model_description_revisions SET label = $3 WHERE id = $2 AND model_id = $1",
        id,
        rev,
        input.label.as_deref(),
    )
    .execute(&state.db)
    .await;
    match result {
        Ok(r) if r.rows_affected() == 0 => Err(ApiError::NotFound),
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Err(ApiError::Conflict(
            "that label is already used on this model".into(),
        )),
        Err(e) => Err(e.into()),
    }
}
