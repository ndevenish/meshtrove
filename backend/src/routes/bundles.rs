//! Bundles: CRUD, full-text search, markdown description revisions, and member
//! model management. Mirrors `models.rs` (bundles have no variants or purchase
//! fields, but add a `kind` and a `bundle_models` membership m2m).

use std::collections::HashSet;

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::custom_fields::{
    CustomFieldValueDetail, CustomFieldValueInput, ValueOwner, apply_values, fetch_values,
    persist_bundle_fields,
};
use crate::routes::models::{
    DescriptionInput, ImageSummary, LabelInput, ModelSummary, Revision, SearchQuery,
};
use crate::routes::tags::upsert_tag;
use crate::state::AppState;
use crate::util::{slug_token, slug_token_of, slugify};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/bundles", get(search).post(create))
        .route("/api/bundles/{id}", get(detail).put(update).delete(remove))
        .route("/api/bundles/{id}/description", put(update_description))
        .route(
            "/api/bundles/{id}/description/revisions",
            get(list_revisions),
        )
        .route(
            "/api/bundles/{id}/description/revisions/{rev}/label",
            put(label_revision),
        )
        .route("/api/bundles/{id}/categories", put(set_categories))
        .route("/api/bundles/{id}/models/tags", post(retag_members))
        .route("/api/bundles/{id}/models", axum::routing::post(add_model))
        .route(
            "/api/bundles/{id}/models/{model_id}",
            axum::routing::delete(remove_model),
        )
}

// ---------------------------------------------------------------------------
// search
// ---------------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
pub struct BundleSummary {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub kind: String,
    pub creator_id: Option<Uuid>,
    pub creator_name: Option<String>,
    pub primary_image_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub model_count: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
pub struct BundleResults {
    pub bundles: Vec<BundleSummary>,
    pub total: i64,
    pub page: u32,
    pub per_page: u32,
}

/// Shared WHERE clauses for a bundle search (no variant opts — bundles have none).
pub fn push_bundle_filters(qb: &mut QueryBuilder<sqlx::Postgres>, q: &str, tags: &[String]) {
    if !q.is_empty() {
        qb.push(" AND (b.search @@ websearch_to_tsquery('english', ")
            .push_bind(q.to_string())
            .push(") OR b.name ILIKE '%' || ")
            .push_bind(q.to_string())
            .push(" || '%')");
    }
    for tag in tags {
        qb.push(" AND EXISTS (SELECT 1 FROM bundle_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.bundle_id = b.id AND t.name = ")
            .push_bind(tag.clone())
            .push(")");
    }
}

/// The shared SELECT list producing a `BundleSummary` (alias `b` for bundles).
const BUNDLE_SUMMARY_COLS: &str = r#"b.id, b.name, b.slug, b.kind::text AS kind, b.creator_id,
    b.updated_at, c.name AS creator_name,
    bundle_preview_image(b.id) AS primary_image_id,
    (SELECT count(*) FROM bundle_models bm WHERE bm.bundle_id = b.id) AS model_count,
    coalesce((SELECT array_agg(t.name::text ORDER BY t.name) FROM bundle_tags bt
              JOIN tags t ON t.id = bt.tag_id WHERE bt.bundle_id = b.id), '{}') AS tags"#;

fn bundle_summary_from_row(row: &sqlx::postgres::PgRow) -> Result<BundleSummary, sqlx::Error> {
    Ok(BundleSummary {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        slug: row.try_get("slug")?,
        kind: row.try_get("kind")?,
        creator_id: row.try_get("creator_id")?,
        creator_name: row.try_get("creator_name")?,
        primary_image_id: row.try_get("primary_image_id")?,
        tags: row.try_get("tags")?,
        model_count: row.try_get("model_count")?,
        updated_at: row.try_get("updated_at")?,
    })
}

async fn search(
    State(state): State<AppState>,
    _user: User,
    Query(query): Query<SearchQuery>,
) -> Result<Json<BundleResults>, ApiError> {
    let q = query.q.unwrap_or_default().trim().to_string();
    let tags: Vec<String> = query
        .tags
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(30).clamp(1, 100);

    let mut count_qb = QueryBuilder::new("SELECT count(*) FROM bundles b WHERE TRUE");
    push_bundle_filters(&mut count_qb, &q, &tags);
    let total: i64 = count_qb.build_query_scalar().fetch_one(&state.db).await?;

    let mut qb = QueryBuilder::new(format!(
        "SELECT {BUNDLE_SUMMARY_COLS} FROM bundles b LEFT JOIN creators c ON c.id = b.creator_id WHERE TRUE"
    ));
    push_bundle_filters(&mut qb, &q, &tags);
    if q.is_empty() {
        qb.push(" ORDER BY b.updated_at DESC");
    } else {
        qb.push(" ORDER BY ts_rank(b.search, websearch_to_tsquery('english', ")
            .push_bind(&q)
            .push(")) DESC, b.updated_at DESC");
    }
    qb.push(" LIMIT ")
        .push_bind(per_page as i64)
        .push(" OFFSET ")
        .push_bind(((page - 1) * per_page) as i64);

    let rows: Vec<sqlx::postgres::PgRow> = qb.build().fetch_all(&state.db).await?;
    let bundles = rows
        .iter()
        .map(bundle_summary_from_row)
        .collect::<Result<_, _>>()
        .context("decoding bundle search row")?;

    Ok(Json(BundleResults {
        bundles,
        total,
        page,
        per_page,
    }))
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct BundleInput {
    pub name: String,
    pub creator_id: Option<Uuid>,
    pub source_url: Option<String>,
    /// 'purchased' (a bought pack) or 'collection' (a personal grouping)
    pub kind: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Initial markdown description (creates revision 1)
    pub description_md: Option<String>,
    /// Admin-defined extra fields to write alongside the built-in ones. Absent
    /// means "leave them alone"; an entry with a null value clears that field.
    pub custom_fields: Option<Vec<CustomFieldValueInput>>,
}

#[derive(Serialize, ToSchema)]
pub struct BundleDetail {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub kind: String,
    pub creator_id: Option<Uuid>,
    pub creator_name: Option<String>,
    pub source_url: Option<String>,
    pub tags: Vec<String>,
    pub description_md: Option<String>,
    /// Every custom field that applies to bundles and that the caller may see,
    /// set or not, in display order.
    pub custom_fields: Vec<CustomFieldValueDetail>,
    pub models: Vec<ModelSummary>,
    pub images: Vec<ImageSummary>,
    /// The bundle's primary categories (import sections), in tab order. A
    /// category is a model tag; a member belongs to it by carrying that tag.
    pub categories: Vec<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn parse_kind(kind: Option<&str>) -> Result<&str, ApiError> {
    match kind.unwrap_or("purchased") {
        k @ ("purchased" | "collection") => Ok(k),
        other => Err(ApiError::BadRequest(format!(
            "unknown bundle kind {other:?}"
        ))),
    }
}

/// A slug for `name`: `slugify(name)` plus a random token (see
/// [`crate::routes::models::unique_slug`]). On a rename, `keep` is the current
/// slug whose token is preserved; `exclude` is the row being updated. Both
/// `None` when creating.
pub async fn unique_slug(
    state: &AppState,
    name: &str,
    keep: Option<&str>,
    exclude: Option<Uuid>,
) -> Result<String, ApiError> {
    let base = slugify(name);
    let mut token = keep.and_then(slug_token_of).map(str::to_string);
    loop {
        let candidate = match &token {
            Some(t) => format!("{base}-{t}"),
            None => format!("{base}-{}", slug_token()),
        };
        let clash = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM bundles
                 WHERE slug = $1 AND ($2::uuid IS NULL OR id <> $2))",
            candidate,
            exclude,
        )
        .fetch_one(&state.db)
        .await?
        .unwrap_or(false);
        if !clash {
            return Ok(candidate);
        }
        token = None;
    }
}

async fn set_bundle_tags(
    tx: &mut sqlx::PgConnection,
    bundle_id: Uuid,
    tags: &[String],
) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM bundle_tags WHERE bundle_id = $1", bundle_id)
        .execute(&mut *tx)
        .await?;
    for tag in tags {
        let tag = upsert_tag(&mut *tx, tag).await?;
        sqlx::query!(
            "INSERT INTO bundle_tags (bundle_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            bundle_id,
            tag.id,
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

pub async fn bundle_created_by(state: &AppState, id: Uuid) -> Result<Uuid, ApiError> {
    sqlx::query_scalar!("SELECT created_by FROM bundles WHERE id = $1", id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<BundleInput>,
) -> Result<Json<BundleDetail>, ApiError> {
    user.require_editor()?;
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    let kind = parse_kind(input.kind.as_deref())?;
    let slug = unique_slug(&state, &name, None, None).await?;

    let mut tx = state.db.begin().await?;
    let bundle_id: Uuid = sqlx::query_scalar!(
        r#"INSERT INTO bundles (name, slug, creator_id, source_url, kind, created_by)
           VALUES ($1, $2, $3, $4, $5::bundle_kind, $6)
           RETURNING id"#,
        name,
        slug,
        input.creator_id,
        input.source_url,
        kind as _,
        user.id,
    )
    .fetch_one(&mut *tx)
    .await?;

    set_bundle_tags(&mut tx, bundle_id, &input.tags).await?;

    if let Some(body) = &input.description_md {
        sqlx::query!(
            "INSERT INTO bundle_description_revisions (bundle_id, body_md, created_by)
             VALUES ($1, $2, $3)",
            bundle_id,
            body,
            user.id,
        )
        .execute(&mut *tx)
        .await?;
    }
    if let Some(values) = &input.custom_fields {
        apply_values(&mut tx, ValueOwner::Bundle(bundle_id), values, &user).await?;
    }
    tx.commit().await?;

    fetch_detail(&state, bundle_id, &user).await.map(Json)
}

/// The member models of a bundle, shaped as ModelSummary for reuse by the UI.
async fn fetch_members(
    state: &AppState,
    bundle_id: Uuid,
    viewer: Uuid,
) -> Result<Vec<ModelSummary>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT m.id, m.name, m.slug, m.creator_id, c.name as "creator_name?", m.updated_at,
                  model_preview_image(m.id) as primary_image_id,
                  (SELECT count(*) FROM user_model_marks k WHERE k.model_id = m.id AND k.mark = 'liked') as "like_count!",
                  EXISTS (SELECT 1 FROM user_model_marks k
                           WHERE k.model_id = m.id AND k.mark = 'liked' AND k.user_id = $2) as "liked!",
                  (SELECT count(*) FROM model_variants v WHERE v.model_id = m.id) as "variant_count!",
                  coalesce((SELECT array_agg(t.name::text ORDER BY t.name) FROM model_tags mt
                            JOIN tags t ON t.id = mt.tag_id WHERE mt.model_id = m.id), '{}') as "tags!"
           FROM models m LEFT JOIN creators c ON c.id = m.creator_id
           WHERE m.id IN (SELECT model_id FROM bundle_models WHERE bundle_id = $1)
           ORDER BY m.name"#,
        bundle_id,
        viewer,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ModelSummary {
            id: r.id,
            name: r.name,
            slug: r.slug,
            creator_id: r.creator_id,
            creator_name: r.creator_name,
            primary_image_id: r.primary_image_id,
            like_count: r.like_count,
            liked: r.liked,
            variant_count: r.variant_count,
            tags: r.tags,
            matched_variant_ids: None,
            updated_at: r.updated_at,
        })
        .collect())
}

async fn fetch_detail(state: &AppState, id: Uuid, user: &User) -> Result<BundleDetail, ApiError> {
    let row = sqlx::query!(
        r#"SELECT b.id, b.name, b.slug, b.kind::text as "kind!", b.creator_id,
                  c.name as "creator_name?", b.source_url, b.created_by, b.created_at, b.updated_at,
                  (SELECT r.body_md FROM bundle_description_revisions r
                    WHERE r.bundle_id = b.id ORDER BY r.created_at DESC LIMIT 1) as description_md,
                  coalesce((SELECT array_agg(t.name::text ORDER BY t.name) FROM bundle_tags bt
                            JOIN tags t ON t.id = bt.tag_id WHERE bt.bundle_id = b.id), '{}') as "tags!"
           FROM bundles b LEFT JOIN creators c ON c.id = b.creator_id
           WHERE b.id = $1"#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let images = sqlx::query!(
        r#"SELECT id, kind::text as "kind!", is_primary, width, height FROM images
           WHERE bundle_id = $1 ORDER BY is_primary DESC, sort_order, created_at"#,
        id,
    )
    .fetch_all(&state.db)
    .await?;

    let models = fetch_members(state, id, user.id).await?;
    let custom_fields = fetch_values(state, ValueOwner::Bundle(id), user).await?;

    let categories: Vec<String> = sqlx::query_scalar!(
        r#"SELECT t.name::text as "name!" FROM bundle_categories bc
           JOIN tags t ON t.id = bc.tag_id
           WHERE bc.bundle_id = $1 ORDER BY bc.position"#,
        id,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(BundleDetail {
        id: row.id,
        name: row.name,
        slug: row.slug,
        kind: row.kind,
        creator_id: row.creator_id,
        creator_name: row.creator_name,
        source_url: row.source_url,
        tags: row.tags,
        description_md: row.description_md,
        custom_fields,
        models,
        images: images
            .into_iter()
            .map(|i| ImageSummary {
                id: i.id,
                kind: i.kind,
                is_primary: i.is_primary,
                width: i.width,
                height: i.height,
                // A bundle's own gallery is its own images; the member models'
                // pictures belong on the member models.
                variant_id: None,
            })
            .collect(),
        categories,
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Resolve a path segment that is either a bundle's UUID or its slug to the id.
/// Canonical URLs use the slug; a UUID still resolves and the client redirects.
pub async fn resolve_id(state: &AppState, key: &str) -> Result<Uuid, ApiError> {
    if let Ok(id) = Uuid::parse_str(key) {
        return Ok(id);
    }
    sqlx::query_scalar!("SELECT id FROM bundles WHERE slug = $1", key)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)
}

async fn detail(
    State(state): State<AppState>,
    user: User,
    Path(key): Path<String>,
) -> Result<Json<BundleDetail>, ApiError> {
    let id = resolve_id(&state, &key).await?;
    fetch_detail(&state, id, &user).await.map(Json)
}

async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<BundleInput>,
) -> Result<Json<BundleDetail>, ApiError> {
    user.require_can_edit(bundle_created_by(&state, id).await?)?;
    let kind = parse_kind(input.kind.as_deref())?;
    // The slug follows the name, keeping its token across the rename (see
    // models::update).
    let name = input.name.trim();
    let current_slug = sqlx::query_scalar!("SELECT slug FROM bundles WHERE id = $1", id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    let slug = unique_slug(&state, name, Some(&current_slug), Some(id)).await?;

    let mut tx = state.db.begin().await?;
    // An edit through the form is a person naming this bundle: from now on it is
    // user-owned, so a metadata import must not overwrite the name.
    sqlx::query!(
        r#"UPDATE bundles SET name = $2, slug = $6, creator_id = $3, source_url = $4,
               kind = $5::bundle_kind, name_autogenerated = false, updated_at = now()
           WHERE id = $1"#,
        id,
        name,
        input.creator_id,
        input.source_url,
        kind as _,
        slug,
    )
    .execute(&mut *tx)
    .await?;
    set_bundle_tags(&mut tx, id, &input.tags).await?;
    if let Some(values) = &input.custom_fields {
        apply_values(&mut tx, ValueOwner::Bundle(id), values, &user).await?;
        // A field marked "persists to model" reaches its members when the
        // bundle's value is written, and only then.
        persist_bundle_fields(&mut tx, id, &user).await?;
    }
    tx.commit().await?;

    fetch_detail(&state, id, &user).await.map(Json)
}

/// What to do with the bundle's member models when the bundle is deleted.
#[derive(Deserialize, ToSchema, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemberDisposition {
    /// Keep every member — they only lose their link to this bundle (the default,
    /// and what a plain DELETE has always done).
    #[default]
    Keep,
    /// Delete every member, including ones that also belong to another bundle.
    Delete,
    /// Delete only the members unique to this bundle; keep any that also belong to
    /// another bundle (those just leave this one).
    DeleteExclusive,
}

#[derive(Deserialize, ToSchema)]
pub struct RemoveParams {
    #[serde(default)]
    pub members: MemberDisposition,
}

async fn remove(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Query(params): Query<RemoveParams>,
) -> Result<StatusCode, ApiError> {
    user.require_can_edit(bundle_created_by(&state, id).await?)?;

    let mut tx = state.db.begin().await?;

    // The members this delete removes (as (id, created_by)), per the disposition.
    // A member is a standalone model with its own owner, so each one deleted is
    // gated on the caller's edit permission *before* any are removed — the delete
    // is all-or-nothing, never a partial wipe. Deleting a model cascades to its
    // variants, files, images and bundle_models links (in this and any other
    // bundle); blobs are content-addressed and swept by GC (docs/plan.md).
    let to_delete: Vec<(Uuid, Uuid)> = match params.members {
        MemberDisposition::Keep => Vec::new(),
        MemberDisposition::Delete => sqlx::query!(
            "SELECT m.id, m.created_by FROM models m
             JOIN bundle_models bm ON bm.model_id = m.id WHERE bm.bundle_id = $1",
            id,
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|r| (r.id, r.created_by))
        .collect(),
        MemberDisposition::DeleteExclusive => sqlx::query!(
            "SELECT m.id, m.created_by FROM models m
             JOIN bundle_models bm ON bm.model_id = m.id
             WHERE bm.bundle_id = $1
               AND NOT EXISTS (
                 SELECT 1 FROM bundle_models other
                 WHERE other.model_id = m.id AND other.bundle_id <> $1
               )",
            id,
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|r| (r.id, r.created_by))
        .collect(),
    };
    for (_, created_by) in &to_delete {
        user.require_can_edit(*created_by)?;
    }
    if !to_delete.is_empty() {
        let ids: Vec<Uuid> = to_delete.iter().map(|(id, _)| *id).collect();
        sqlx::query!("DELETE FROM models WHERE id = ANY($1::uuid[])", &ids)
            .execute(&mut *tx)
            .await?;
    }
    // Members not deleted just lose their bundle_models link (cascade); the bundle
    // row itself goes either way.
    sqlx::query!("DELETE FROM bundles WHERE id = $1", id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// membership
// ---------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct AddModelInput {
    pub model_id: Uuid,
}

async fn add_model(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<AddModelInput>,
) -> Result<StatusCode, ApiError> {
    user.require_can_edit(bundle_created_by(&state, id).await?)?;
    let result = sqlx::query!(
        "INSERT INTO bundle_models (bundle_id, model_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        id,
        input.model_id,
    )
    .execute(&state.db)
    .await;
    match result {
        Ok(_) => {}
        // FK violation = no such model
        Err(sqlx::Error::Database(e)) if e.is_foreign_key_violation() => {
            return Err(ApiError::BadRequest("no such model".into()));
        }
        Err(e) => return Err(e.into()),
    }
    sqlx::query!("UPDATE bundles SET updated_at = now() WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_model(
    State(state): State<AppState>,
    user: User,
    Path((id, model_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    user.require_can_edit(bundle_created_by(&state, id).await?)?;
    let result = sqlx::query!(
        "DELETE FROM bundle_models WHERE bundle_id = $1 AND model_id = $2",
        id,
        model_id,
    )
    .execute(&state.db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    sqlx::query!("UPDATE bundles SET updated_at = now() WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// categories (the bundle's ordered sections)
// ---------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct CategoriesInput {
    /// The full ordered list of category (model-tag) names. Replaces whatever
    /// the bundle had — so reorder, add and remove all come through as one array.
    pub categories: Vec<String>,
}

/// Rewrite a bundle's category list from the curation UI. Each name resolves to
/// a model tag (created if new); `position` follows the array. A category is a
/// tag a member may carry — this endpoint only records *which* tags are sections
/// and in what order, it never tags or untags any model.
async fn set_categories(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<CategoriesInput>,
) -> Result<Json<BundleDetail>, ApiError> {
    user.require_can_edit(bundle_created_by(&state, id).await?)?;
    let mut tx = state.db.begin().await?;
    sqlx::query!("DELETE FROM bundle_categories WHERE bundle_id = $1", id)
        .execute(&mut *tx)
        .await?;
    let mut seen: HashSet<Uuid> = HashSet::new();
    let mut position: i32 = 0;
    for name in &input.categories {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let tag = upsert_tag(&mut *tx, name).await?;
        // Two spellings of one tag collapse to a single section (keep the first).
        if !seen.insert(tag.id) {
            continue;
        }
        sqlx::query!(
            "INSERT INTO bundle_categories (bundle_id, tag_id, position) VALUES ($1, $2, $3)",
            id,
            tag.id,
            position,
        )
        .execute(&mut *tx)
        .await?;
        position += 1;
    }
    sqlx::query!("UPDATE bundles SET updated_at = now() WHERE id = $1", id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    fetch_detail(&state, id, &user).await.map(Json)
}

// ---------------------------------------------------------------------------
// bulk member retag
// ---------------------------------------------------------------------------

/// Tags to add to, and remove from, every member model at once.
///
/// Additive/subtractive on purpose — there is no "replace" mode. Members reach a
/// bundle carrying their own tags from import and scraped patches, and a bundle
/// is a shipping crate, not a category: a wholesale overwrite would silently
/// discard per-model tags the user cannot get back. Add and remove compose well
/// enough to reach any state deliberately.
#[derive(Deserialize, ToSchema)]
pub struct MemberTagsInput {
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
}

/// What the retag actually did — assignment counts, not tag counts, so
/// "3 tags across 12 models" reads as 36 and re-running reads as 0.
#[derive(Serialize, ToSchema)]
pub struct MemberTagsResult {
    /// Members whose tag set changed at all.
    pub models_updated: i64,
    pub tags_added: i64,
    pub tags_removed: i64,
}

async fn retag_members(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<MemberTagsInput>,
) -> Result<Json<MemberTagsResult>, ApiError> {
    user.require_can_edit(bundle_created_by(&state, id).await?)?;

    let mut tx = state.db.begin().await?;

    // Resolve the adds first, inside this transaction — `upsert_tag` takes FK
    // locks the later INSERT needs, and doing it on a pooled connection while
    // this transaction is open would deadlock against ourselves.
    let mut add_ids: Vec<Uuid> = Vec::new();
    for name in &input.add {
        if name.trim().is_empty() {
            continue;
        }
        let tag = upsert_tag(&mut *tx, name).await?;
        if !add_ids.contains(&tag.id) {
            add_ids.push(tag.id);
        }
    }

    // Removals resolve by name against existing tags only: removing a tag that
    // was never created is a no-op, not a reason to mint it.
    let remove_names: Vec<String> = input
        .remove
        .iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();

    // A tag named in both lists would race itself — the add wins, so a UI that
    // lets the user do both by accident still lands somewhere predictable.
    let removed = sqlx::query_scalar!(
        r#"WITH doomed AS (
               SELECT t.id FROM tags t
                WHERE t.name = ANY($2::citext[]) AND NOT (t.id = ANY($3::uuid[]))
           )
           DELETE FROM model_tags mt
            USING bundle_models bm
            WHERE mt.model_id = bm.model_id
              AND bm.bundle_id = $1
              AND mt.tag_id IN (SELECT id FROM doomed)
           RETURNING mt.model_id"#,
        id,
        &remove_names as &[String],
        &add_ids,
    )
    .fetch_all(&mut *tx)
    .await?;

    let added = sqlx::query_scalar!(
        r#"INSERT INTO model_tags (model_id, tag_id)
           SELECT bm.model_id, t.id
             FROM bundle_models bm CROSS JOIN unnest($2::uuid[]) AS t (id)
            WHERE bm.bundle_id = $1
           ON CONFLICT DO NOTHING
           RETURNING model_id"#,
        id,
        &add_ids,
    )
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;

    let touched: HashSet<Uuid> = added.iter().chain(removed.iter()).copied().collect();
    Ok(Json(MemberTagsResult {
        models_updated: touched.len() as i64,
        tags_added: added.len() as i64,
        tags_removed: removed.len() as i64,
    }))
}

// ---------------------------------------------------------------------------
// description revisions (mirror of models', retargeted to bundles)
// ---------------------------------------------------------------------------

async fn update_description(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<DescriptionInput>,
) -> Result<Json<Revision>, ApiError> {
    user.require_can_edit(bundle_created_by(&state, id).await?)?;
    let row = sqlx::query!(
        r#"INSERT INTO bundle_description_revisions (bundle_id, body_md, created_by)
           VALUES ($1, $2, $3)
           RETURNING id, body_md, label as "label: String", created_by, created_at"#,
        id,
        input.body_md,
        user.id,
    )
    .fetch_one(&state.db)
    .await?;
    sqlx::query!("UPDATE bundles SET updated_at = now() WHERE id = $1", id)
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
           FROM bundle_description_revisions r JOIN users u ON u.id = r.created_by
           WHERE r.bundle_id = $1 ORDER BY r.created_at DESC"#,
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

async fn label_revision(
    State(state): State<AppState>,
    user: User,
    Path((id, rev)): Path<(Uuid, Uuid)>,
    Json(input): Json<LabelInput>,
) -> Result<StatusCode, ApiError> {
    user.require_can_edit(bundle_created_by(&state, id).await?)?;
    let result = sqlx::query!(
        "UPDATE bundle_description_revisions SET label = $3 WHERE id = $2 AND bundle_id = $1",
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
            "that label is already used on this bundle".into(),
        )),
        Err(e) => Err(e.into()),
    }
}
