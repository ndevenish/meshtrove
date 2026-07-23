//! Models: CRUD, unified search (text + tags + variant tags), and markdown
//! description revisions.

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
use crate::routes::custom_fields::{
    CustomFieldValueDetail, CustomFieldValueInput, ValueOwner, apply_values, copy_values_onto,
    fetch_values, values_of,
};
use crate::routes::tags::upsert_tag;
use crate::routes::variants::{
    VariantDetail, fetch_variants, set_variant_tags, variant_with_tag_set,
};
use crate::state::AppState;
use crate::util::{slug_token, slug_token_of, slugify};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/models", get(search).post(create))
        .route("/api/models/{id}", get(detail).put(update).delete(remove))
        .route("/api/models/{id}/merge", axum::routing::post(merge))
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
    /// Comma-separated variant tag names; a single variant must carry all of them
    pub vtags: Option<String>,
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
    /// whether the *calling* user has liked it — what the heart button renders
    pub liked: bool,
    pub variant_count: i64,
    /// Variants satisfying the `vtags` filter (when one was given)
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

/// Split a comma-separated query parameter into trimmed, non-empty names.
pub fn parse_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Require one variant of `v` to carry every named variant tag at once. A model
/// with a 32mm+supported variant and a 75mm+unsupported one does NOT match
/// `32mm + unsupported`. Called with the `model_variants v` correlation already
/// open, so it appends `AND EXISTS (…)` clauses tied to that same `v.id`. The
/// join alias is `ft` (not `t`) so callers can reference their own outer `t`.
pub fn push_variant_tag_filters(qb: &mut QueryBuilder<sqlx::Postgres>, vtags: &[String]) {
    for tag in vtags {
        qb.push(
            " AND EXISTS (SELECT 1 FROM variant_tag_assignments a
                          JOIN variant_tags ft ON ft.id = a.tag_id
                         WHERE a.variant_id = v.id AND ft.name = ",
        )
        .push_bind(tag.clone())
        .push(")");
    }
}

/// Full-text + fuzzy-name predicate (alias `m`).
pub fn push_text_filter(qb: &mut QueryBuilder<sqlx::Postgres>, q: &str) {
    if !q.is_empty() {
        qb.push(" AND (m.search @@ websearch_to_tsquery('english', ")
            .push_bind(q.to_string())
            .push(") OR m.name ILIKE '%' || ")
            .push_bind(q.to_string())
            .push(" || '%')");
    }
}

/// Require the model (alias `m`) to carry every named tag. The join alias is
/// `ft` so a caller correlating to an outer `t` (e.g. the tag-cloud count) is
/// not shadowed.
pub fn push_model_tag_filters(qb: &mut QueryBuilder<sqlx::Postgres>, tags: &[String]) {
    for tag in tags {
        qb.push(" AND EXISTS (SELECT 1 FROM model_tags mt JOIN tags ft ON ft.id = mt.tag_id WHERE mt.model_id = m.id AND ft.name = ")
            .push_bind(tag.clone())
            .push(")");
    }
}

/// Require the model (alias `m`) to have one variant carrying every named
/// variant tag at once.
pub fn push_variant_group(qb: &mut QueryBuilder<sqlx::Postgres>, vtags: &[String]) {
    if !vtags.is_empty() {
        qb.push(" AND EXISTS (SELECT 1 FROM model_variants v WHERE v.model_id = m.id");
        push_variant_tag_filters(qb, vtags);
        qb.push(")");
    }
}

/// Append the shared WHERE clauses for a search to a query builder (alias `m`).
pub fn push_filters(
    qb: &mut QueryBuilder<sqlx::Postgres>,
    q: &str,
    tags: &[String],
    vtags: &[String],
) {
    push_text_filter(qb, q);
    push_model_tag_filters(qb, tags);
    push_variant_group(qb, vtags);
}

async fn search(
    State(state): State<AppState>,
    user: User,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResults>, ApiError> {
    let q = query.q.unwrap_or_default().trim().to_string();
    let tags = parse_csv(&query.tags.unwrap_or_default());
    let vtags = parse_csv(&query.vtags.unwrap_or_default());
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(30).clamp(1, 100);

    let mut count_qb = QueryBuilder::new("SELECT count(*) FROM models m WHERE TRUE");
    push_filters(&mut count_qb, &q, &tags, &vtags);
    let total: i64 = count_qb.build_query_scalar().fetch_one(&state.db).await?;

    let mut qb = QueryBuilder::new(
        r#"SELECT m.id, m.name, m.slug, m.creator_id, m.updated_at, c.name AS creator_name,
              model_preview_image(m.id) AS primary_image_id,
              (SELECT count(*) FROM user_model_marks k WHERE k.model_id = m.id AND k.mark = 'liked') AS like_count,
              (SELECT count(*) FROM model_variants v WHERE v.model_id = m.id) AS variant_count,
              coalesce((SELECT array_agg(t.name::text ORDER BY t.name) FROM model_tags mt
                        JOIN tags t ON t.id = mt.tag_id WHERE mt.model_id = m.id), '{}') AS tags,
              EXISTS (SELECT 1 FROM user_model_marks k WHERE k.model_id = m.id AND k.mark = 'liked' AND k.user_id = "#,
    );
    qb.push_bind(user.id).push(
        r#") AS liked
         FROM models m LEFT JOIN creators c ON c.id = m.creator_id WHERE TRUE"#,
    );
    push_filters(&mut qb, &q, &tags, &vtags);
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
                    liked: row.try_get("liked")?,
                    variant_count: row.try_get("variant_count")?,
                    tags: row.try_get("tags")?,
                    matched_variant_ids: None,
                    updated_at: row.try_get("updated_at")?,
                })
            },
        )
        .collect::<Result<_, _>>()
        .context("decoding search row")?;

    // Mark which variants matched the vtags filter so the UI can highlight them.
    if !vtags.is_empty() && !models.is_empty() {
        let model_ids: Vec<Uuid> = models.iter().map(|m| m.id).collect();
        let mut vq = QueryBuilder::new(
            "SELECT v.id, v.model_id FROM model_variants v WHERE v.model_id = ANY(",
        );
        vq.push_bind(model_ids).push(")");
        push_variant_tag_filters(&mut vq, &vtags);
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
    /// The creator's own id/SKU for this model — free text, not the creators FK.
    pub creator_ref: Option<String>,
    /// The creator's version for this model — free text ("v2", "2024 rework").
    pub model_version: Option<String>,
    pub source_url: Option<String>,
    pub license: Option<String>,
    pub purchase_price: Option<f64>,
    pub purchase_date: Option<NaiveDate>,
    pub order_ref: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Initial markdown description (creates revision 1)
    pub description_md: Option<String>,
    /// Admin-defined extra fields to write alongside the built-in ones. Absent
    /// means "leave them alone"; an entry with a null value clears that field.
    pub custom_fields: Option<Vec<CustomFieldValueInput>>,
}

#[derive(Serialize, ToSchema)]
pub struct ModelDetail {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub creator_id: Option<Uuid>,
    pub creator_name: Option<String>,
    pub creator_ref: Option<String>,
    pub model_version: Option<String>,
    pub source_url: Option<String>,
    pub license: Option<String>,
    pub purchase_price: Option<f64>,
    pub purchase_date: Option<NaiveDate>,
    pub order_ref: Option<String>,
    pub tags: Vec<String>,
    pub description_md: Option<String>,
    /// Every custom field that applies to models and that the caller may see,
    /// set or not, in display order.
    pub custom_fields: Vec<CustomFieldValueDetail>,
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
    pub slug: String,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct ImageSummary {
    pub id: Uuid,
    pub kind: String,
    pub is_primary: bool,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Set when the image belongs to one of the model's variants rather than to
    /// the model itself — the gallery shows both, but "primary" means a different
    /// thing for each, and the UI has to be able to tell them apart.
    pub variant_id: Option<Uuid>,
}

/// A slug for `name`: `slugify(name)` plus a random token, so no model is
/// privileged with the plain slug by being created first. On a rename, `keep` is
/// the row's current slug, whose token is preserved so the URL keeps its identity
/// as the name changes; `exclude` is that row, skipped in the uniqueness check.
/// Both are `None` when creating.
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
            "SELECT EXISTS(SELECT 1 FROM models
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
        // A kept token collided (astronomically unlikely) — mint a fresh one.
        token = None;
    }
}

async fn set_model_tags(
    tx: &mut sqlx::PgConnection,
    model_id: Uuid,
    tags: &[String],
) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM model_tags WHERE model_id = $1", model_id)
        .execute(&mut *tx)
        .await?;
    for tag in tags {
        let tag = upsert_tag(&mut *tx, tag).await?;
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
    let slug = unique_slug(&state, &name, None, None).await?;

    let mut tx = state.db.begin().await?;
    let model_id: Uuid = sqlx::query_scalar!(
        r#"INSERT INTO models (name, slug, creator_id, creator_ref, model_version, source_url, license,
                               purchase_price, purchase_date, order_ref, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8::float8::numeric(10,2), $9, $10, $11)
           RETURNING id"#,
        name,
        slug,
        input.creator_id,
        input.creator_ref,
        input.model_version,
        input.source_url,
        input.license,
        input.purchase_price,
        input.purchase_date,
        input.order_ref,
        user.id,
    )
    .fetch_one(&mut *tx)
    .await?;

    set_model_tags(&mut tx, model_id, &input.tags).await?;

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
    if let Some(values) = &input.custom_fields {
        apply_values(&mut tx, ValueOwner::Model(model_id), values, &user).await?;
    }
    tx.commit().await?;

    fetch_detail(&state, model_id, &user).await.map(Json)
}

async fn fetch_detail(state: &AppState, id: Uuid, user: &User) -> Result<ModelDetail, ApiError> {
    let row = sqlx::query!(
        r#"SELECT m.id, m.name, m.slug, m.creator_id, c.name as "creator_name?",
                  m.creator_ref, m.model_version,
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

    // A model's pictures are its variants' pictures too. A carve puts the STLs on
    // a variant, so every render of a carved model is an image *of a variant* —
    // ask only for `model_id = $1` and a model with forty rendered previews shows
    // "No images yet". The model's own images come first (an uploaded shot of the
    // whole thing beats a render of one variant of it), then the variants' in
    // variant order.
    // Every variant's thumbnail belongs in the model's gallery — a carve renders
    // one picture per variant, and they are all pictures of this model. The
    // model's own images lead (an uploaded shot, or one promoted from a variant);
    // the variants follow plainest-first, matching `model_preview_image` exactly
    // so the card and the top of the gallery are never two different pictures.
    let images = sqlx::query!(
        r#"SELECT i.id, i.kind::text as "kind!", i.is_primary, i.width, i.height,
                  i.variant_id,
                  (i.model_id IS NULL) as "from_variant!",
                  coalesce(
                      (SELECT count(*) FROM variant_tag_assignments a
                       WHERE a.variant_id = i.variant_id), 0) as "tag_count!",
                  coalesce(length(f.filename), 0) as "name_len!"
           FROM images i
           LEFT JOIN model_variants v ON v.id = i.variant_id
           LEFT JOIN files f ON f.id = i.source_file_id
           WHERE (i.model_id = $1 OR v.model_id = $1)
             -- A promoted picture is the model's now; don't also show the
             -- variant's copy of the identical blob back at the user.
             AND NOT (i.model_id IS NULL AND EXISTS (
                 SELECT 1 FROM images own
                 WHERE own.model_id = $1 AND own.blob_sha256 = i.blob_sha256))
           ORDER BY "from_variant!", i.is_primary DESC, "tag_count!", "name_len!",
                    i.sort_order, i.created_at"#,
        id,
    )
    .fetch_all(&state.db)
    .await?;

    let variants = fetch_variants(state, id).await?;
    let custom_fields = fetch_values(state, ValueOwner::Model(id), user).await?;

    let bundles = sqlx::query_as!(
        BundleRef,
        r#"SELECT b.id, b.slug, b.name FROM bundles b
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
        creator_ref: row.creator_ref,
        model_version: row.model_version,
        source_url: row.source_url,
        license: row.license,
        purchase_price: row.purchase_price,
        purchase_date: row.purchase_date,
        order_ref: row.order_ref,
        tags: row.tags,
        description_md: row.description_md,
        custom_fields,
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
                variant_id: i.variant_id,
            })
            .collect(),
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Resolve a path segment that is either a model's UUID or its slug to the id.
/// Canonical URLs use the slug; a UUID still resolves (the client redirects it
/// to the slug), and so does an id typed by hand or held in an old bookmark.
pub async fn resolve_id(state: &AppState, key: &str) -> Result<Uuid, ApiError> {
    if let Ok(id) = Uuid::parse_str(key) {
        return Ok(id);
    }
    sqlx::query_scalar!("SELECT id FROM models WHERE slug = $1", key)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)
}

async fn detail(
    State(state): State<AppState>,
    user: User,
    Path(key): Path<String>,
) -> Result<Json<ModelDetail>, ApiError> {
    let id = resolve_id(&state, &key).await?;
    fetch_detail(&state, id, &user).await.map(Json)
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

    // The slug follows the name: a rename gives a new URL, and the old UUID (or
    // old slug, if bookmarked) still resolves and redirects here. The random
    // token is carried over from the current slug, so the URL keeps its identity
    // across the rename; `exclude` skips this row so a no-op rename is stable.
    let name = input.name.trim();
    let current_slug = sqlx::query_scalar!("SELECT slug FROM models WHERE id = $1", id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    let slug = unique_slug(&state, name, Some(&current_slug), Some(id)).await?;

    let mut tx = state.db.begin().await?;
    // The purchase fields coalesce; the rest replace. No editor asks for a licence
    // or a price any more, so an omitted one means "I wasn't told about this",
    // not "erase it" — and a plain rename would otherwise quietly strip the
    // licence and the price off an imported model. Name, creator and source_url
    // are on the form, so a cleared one there is a real instruction to clear.
    sqlx::query!(
        r#"UPDATE models SET name = $2, slug = $9, creator_id = $3, source_url = $4,
               creator_ref = $10, model_version = $11,
               license = coalesce($5, license),
               purchase_price = coalesce($6::float8::numeric(10,2), purchase_price),
               purchase_date = coalesce($7, purchase_date),
               order_ref = coalesce($8, order_ref),
               updated_at = now()
           WHERE id = $1"#,
        id,
        name,
        input.creator_id,
        input.source_url,
        input.license,
        input.purchase_price,
        input.purchase_date,
        input.order_ref,
        slug,
        input.creator_ref,
        input.model_version,
    )
    .execute(&mut *tx)
    .await?;
    set_model_tags(&mut tx, id, &input.tags).await?;
    if let Some(values) = &input.custom_fields {
        apply_values(&mut tx, ValueOwner::Model(id), values, &user).await?;
    }
    tx.commit().await?;

    fetch_detail(&state, id, &user).await.map(Json)
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
// merge
// ---------------------------------------------------------------------------

/// What becomes of the model being merged *in*, once its contents are on the
/// model we're editing.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OtherModel {
    /// Everything comes across — files, variants, pictures, tags, provenance,
    /// likes, bundle memberships and any custom field this model hasn't answered
    /// — and the emptied model is deleted.
    #[default]
    Delete,
    /// It stays exactly as it is; this model gains a *copy* of its files,
    /// variants and pictures (and its tags, and any blank custom field). The
    /// blobs are content-addressed and shared, so the copy is rows, not bytes.
    Keep,
}

#[derive(Deserialize, ToSchema)]
pub struct MergeInput {
    /// The model being merged into this one.
    pub from: Uuid,
    #[serde(default)]
    pub other: OtherModel,
}

/// Fold another model into the one being edited.
///
/// This model survives — it keeps its own name, slug, URL and description. What
/// the caller says is what becomes of the other: a duplicate picked up twice
/// disappears into this one (`delete`), while a model that should stay standing
/// lends this one a copy of its contents (`keep`).
async fn merge(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<MergeInput>,
) -> Result<Json<ModelDetail>, ApiError> {
    user.require_can_edit(model_created_by(&state, id).await?)?;
    if input.from == id {
        return Err(ApiError::BadRequest(
            "a model can't be merged into itself".into(),
        ));
    }
    // Taking another model's files — and, on `delete`, the model itself — needs
    // the right to edit it, not just the right to edit this one.
    user.require_can_edit(model_created_by(&state, input.from).await?)?;

    let mut tx = state.db.begin().await?;
    match input.other {
        OtherModel::Delete => move_model(&mut tx, input.from, id, &user).await?,
        OtherModel::Keep => copy_model(&mut tx, input.from, id, &user).await?,
    }
    sqlx::query!("UPDATE models SET updated_at = now() WHERE id = $1", id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    tracing::info!(into = %id, from = %input.from, other = ?input.other, "merged a model");
    fetch_detail(&state, id, &user).await.map(Json)
}

/// Move everything `from` owns onto `into`, then delete `from`. A variant merge
/// one level up: files and pictures change owner, variants move, and any that
/// then share a tag set with one already on `into` fold together.
async fn move_model(
    tx: &mut sqlx::PgConnection,
    from: Uuid,
    into: Uuid,
    user: &User,
) -> Result<(), ApiError> {
    // Loose model files and the archives they were carved from.
    sqlx::query!(
        "UPDATE files SET model_id = $1 WHERE model_id = $2",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE source_archives SET model_id = $1 WHERE model_id = $2",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;
    // One primary per model (a unique index): this model keeps its own cover, so
    // an incoming primary arrives as an ordinary picture.
    sqlx::query!(
        "UPDATE images SET is_primary = false
          WHERE model_id = $2 AND is_primary
            AND EXISTS (SELECT 1 FROM images o WHERE o.model_id = $1 AND o.is_primary)",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE images SET model_id = $1 WHERE model_id = $2",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;

    // A name is a label, unique per model; a variant's identity is its tag set.
    // Clear an incoming name that collides with one already here so the move
    // can't trip the name index — the tag set that identifies the variant is
    // left untouched.
    sqlx::query!(
        "UPDATE model_variants f SET name = NULL
          WHERE f.model_id = $2 AND f.name IS NOT NULL
            AND EXISTS (SELECT 1 FROM model_variants t
                         WHERE t.model_id = $1 AND t.name = f.name)",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;
    // Move the variants across. The tag-set unique constraint is deferred, so a
    // duplicate can exist until the fold below clears it before commit.
    sqlx::query!(
        "UPDATE model_variants SET model_id = $1 WHERE model_id = $2",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;
    // Two variants of one model with the same tag set were always one variant:
    // fold the pairs the move just created (the same helper the retag path runs).
    sqlx::query!("SELECT merge_duplicate_variants()")
        .execute(&mut *tx)
        .await?;

    // Tags, likes and bundle memberships are sets — union them on.
    sqlx::query!(
        "INSERT INTO model_tags (model_id, tag_id)
         SELECT $1, tag_id FROM model_tags WHERE model_id = $2
         ON CONFLICT DO NOTHING",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO user_model_marks (user_id, model_id, mark)
         SELECT user_id, $1, mark FROM user_model_marks WHERE model_id = $2
         ON CONFLICT DO NOTHING",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO bundle_models (bundle_id, model_id)
         SELECT bundle_id, $1 FROM bundle_models WHERE model_id = $2
         ON CONFLICT DO NOTHING",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;

    fill_blank_fields(&mut *tx, from, into, user).await?;

    // The emptied model goes; its description revisions cascade with it, and this
    // model keeps its own.
    sqlx::query!("DELETE FROM models WHERE id = $1", from)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Copy everything `from` owns onto `into`, leaving `from` untouched. The blobs
/// are content-addressed and shared, so this duplicates rows, not bytes. Likes
/// and bundle memberships are the other model's own record rather than its
/// contents, so they stay with it.
async fn copy_model(
    tx: &mut sqlx::PgConnection,
    from: Uuid,
    into: Uuid,
    user: &User,
) -> Result<(), ApiError> {
    // Loose model files and the record of the archives they came from.
    sqlx::query!(
        "INSERT INTO files (blob_sha256, model_id, path, filename, mime, kind)
         SELECT blob_sha256, $1, path, filename, mime, kind FROM files WHERE model_id = $2",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO source_archives (model_id, filename, sha256, size)
         SELECT $1, filename, sha256, size FROM source_archives WHERE model_id = $2",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;
    // A copied picture points at no source file of its own (that file is on the
    // other model), and an incoming primary only takes if this model has no
    // cover yet.
    sqlx::query!(
        "INSERT INTO images (blob_sha256, model_id, kind, renderer, renderer_config,
                             mime, width, height, is_primary, sort_order, created_by)
         SELECT blob_sha256, $1, kind, renderer, renderer_config, mime, width, height,
                is_primary AND NOT EXISTS
                    (SELECT 1 FROM images o WHERE o.model_id = $1 AND o.is_primary),
                sort_order, created_by
         FROM images WHERE model_id = $2",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;

    // Each of the other model's variants copies onto the one here that shares its
    // tag set, or a fresh one carrying the same tags. Oldest first, so a copied
    // variant that has to be renamed away from a clash keeps the earliest label.
    let variants = sqlx::query!(
        "SELECT id, name, print_notes FROM model_variants
          WHERE model_id = $1 ORDER BY created_at, id",
        from,
    )
    .fetch_all(&mut *tx)
    .await?;
    for v in variants {
        let tag_ids: Vec<Uuid> = sqlx::query_scalar!(
            "SELECT tag_id FROM variant_tag_assignments WHERE variant_id = $1",
            v.id,
        )
        .fetch_all(&mut *tx)
        .await?;

        let target = match variant_with_tag_set(&mut *tx, into, &tag_ids).await? {
            Some(existing) => existing,
            None => {
                let name = free_variant_name(&mut *tx, into, v.name).await?;
                let new_id: Uuid = sqlx::query_scalar!(
                    "INSERT INTO model_variants (model_id, name, print_notes, created_by)
                     VALUES ($1, $2, $3, $4) RETURNING id",
                    into,
                    name,
                    v.print_notes,
                    user.id,
                )
                .fetch_one(&mut *tx)
                .await?;
                set_variant_tags(&mut *tx, new_id, &tag_ids).await?;
                new_id
            }
        };

        sqlx::query!(
            "INSERT INTO files (blob_sha256, variant_id, path, filename, mime, kind)
             SELECT blob_sha256, $1, path, filename, mime, kind FROM files WHERE variant_id = $2",
            target,
            v.id,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "INSERT INTO images (blob_sha256, variant_id, kind, renderer, renderer_config,
                                 mime, width, height, is_primary, sort_order, created_by)
             SELECT blob_sha256, $1, kind, renderer, renderer_config, mime, width, height,
                    is_primary AND NOT EXISTS
                        (SELECT 1 FROM images o WHERE o.variant_id = $1 AND o.is_primary),
                    sort_order, created_by
             FROM images WHERE variant_id = $2",
            target,
            v.id,
        )
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query!(
        "INSERT INTO model_tags (model_id, tag_id)
         SELECT $1, tag_id FROM model_tags WHERE model_id = $2
         ON CONFLICT DO NOTHING",
        into,
        from,
    )
    .execute(&mut *tx)
    .await?;

    fill_blank_fields(&mut *tx, from, into, user).await?;
    Ok(())
}

/// The other model's custom-field answers, copied onto this one, but only for
/// fields this model hasn't answered itself: a value already here wins.
async fn fill_blank_fields(
    tx: &mut sqlx::PgConnection,
    from: Uuid,
    into: Uuid,
    user: &User,
) -> Result<(), ApiError> {
    let theirs = values_of(&mut *tx, ValueOwner::Model(from)).await?;
    let mine = values_of(&mut *tx, ValueOwner::Model(into)).await?;
    copy_values_onto(
        &mut *tx,
        &theirs,
        ValueOwner::Model(into),
        |v| !mine.iter().any(|m| m.field_id == v.field_id),
        user,
    )
    .await?;
    Ok(())
}

/// A variant name free to use on `model_id` — the given one if no variant there
/// already carries it, else none. A name is only a label, so a copied variant
/// gives its up rather than collide (its tag set is what identifies it).
async fn free_variant_name(
    tx: &mut sqlx::PgConnection,
    model_id: Uuid,
    name: Option<String>,
) -> Result<Option<String>, ApiError> {
    let Some(name) = name else { return Ok(None) };
    let taken = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM model_variants WHERE model_id = $1 AND name = $2)",
        model_id,
        name,
    )
    .fetch_one(&mut *tx)
    .await?
    .unwrap_or(false);
    Ok((!taken).then_some(name))
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
