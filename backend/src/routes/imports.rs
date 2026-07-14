//! Imports: the staging area a dropped archive lands in.
//!
//! An import is neither a model nor a bundle — it never shows up in browse or
//! search, only on the "Importing" list. Files uploaded to it are owned by it
//! (`files.import_id`), so a zip can upload and unpack with no decision made
//! about what it *is*. `POST /api/imports/{id}/commit` then moves every staged
//! file onto exactly one destination — a new model, a new bundle, or an existing
//! bundle — and drops the import row.

use std::collections::HashSet;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::bundles::{self, parse_kind as parse_bundle_kind};
use crate::routes::models;
use crate::routes::tags::upsert_tag;
use crate::routes::variant_tags::upsert_variant_tag;
use crate::routes::variants::{set_variant_tags, variant_with_tag_set};
use crate::services::gc;
use crate::services::layout::{self, CarveTarget, LayoutSpec, Plan, PlanVariant};
use crate::state::AppState;
use crate::util::slugify;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/imports", get(list).post(create))
        .route("/api/imports/{id}", get(detail).put(update).delete(remove))
        .route("/api/imports/{id}/plan", post(plan))
        .route("/api/imports/{id}/commit", post(commit))
}

#[derive(Deserialize, ToSchema)]
pub struct ImportInput {
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct ImportSummary {
    pub id: Uuid,
    pub name: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    /// Files staged so far (the archive itself plus anything unpacked from it).
    pub file_count: i64,
    /// An unpack job for one of this import's archives is queued or running:
    /// the contents aren't final yet, so committing is refused.
    pub unpacking: bool,
}

/// An import's files are listed via `GET /api/imports/{id}/files` (files.rs),
/// like every other file owner.
async fn list(
    State(state): State<AppState>,
    user: User,
) -> Result<Json<Vec<ImportSummary>>, ApiError> {
    user.require_editor()?;
    let rows = sqlx::query!(
        r#"SELECT i.id, i.name, i.created_by, i.created_at,
                  (SELECT count(*) FROM files f WHERE f.import_id = i.id) as "file_count!",
                  EXISTS (
                    SELECT 1 FROM jobs j JOIN files f ON f.import_id = i.id
                    WHERE j.kind = 'import_archive'
                      AND j.status IN ('queued', 'running')
                      AND j.payload->>'archive_file_id' = f.id::text
                  ) as "unpacking!"
           FROM imports i
           ORDER BY i.created_at DESC"#,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ImportSummary {
                id: r.id,
                name: r.name,
                created_by: r.created_by,
                created_at: r.created_at,
                file_count: r.file_count,
                unpacking: r.unpacking,
            })
            .collect(),
    ))
}

async fn fetch_import(state: &AppState, id: Uuid) -> Result<ImportSummary, ApiError> {
    let r = sqlx::query!(
        r#"SELECT i.id, i.name, i.created_by, i.created_at,
                  (SELECT count(*) FROM files f WHERE f.import_id = i.id) as "file_count!",
                  EXISTS (
                    SELECT 1 FROM jobs j JOIN files f ON f.import_id = i.id
                    WHERE j.kind = 'import_archive'
                      AND j.status IN ('queued', 'running')
                      AND j.payload->>'archive_file_id' = f.id::text
                  ) as "unpacking!"
           FROM imports i WHERE i.id = $1"#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(ImportSummary {
        id: r.id,
        name: r.name,
        created_by: r.created_by,
        created_at: r.created_at,
        file_count: r.file_count,
        unpacking: r.unpacking,
    })
}

async fn detail(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<ImportSummary>, ApiError> {
    Ok(Json(fetch_import(&state, id).await?))
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<ImportInput>,
) -> Result<Json<ImportSummary>, ApiError> {
    user.require_editor()?;
    let name = input.name.trim();
    let name = if name.is_empty() { "Import" } else { name };
    let id: Uuid = sqlx::query_scalar!(
        "INSERT INTO imports (name, created_by) VALUES ($1, $2) RETURNING id",
        name,
        user.id,
    )
    .fetch_one(&state.db)
    .await?;
    Ok(Json(fetch_import(&state, id).await?))
}

async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<ImportInput>,
) -> Result<Json<ImportSummary>, ApiError> {
    user.require_can_edit(import_created_by(&state, id).await?)?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    sqlx::query!(
        "UPDATE imports SET name = $2, updated_at = now() WHERE id = $1",
        id,
        name,
    )
    .execute(&state.db)
    .await?;
    Ok(Json(fetch_import(&state, id).await?))
}

/// Discard a staged import: the file rows cascade away. The blobs stay in the
/// content-addressed store (they may be shared) — orphan GC is separate.
async fn remove(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    user.require_can_edit(import_created_by(&state, id).await?)?;
    sqlx::query!("DELETE FROM imports WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn import_created_by(state: &AppState, id: Uuid) -> Result<Uuid, ApiError> {
    sqlx::query_scalar!("SELECT created_by FROM imports WHERE id = $1", id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)
}

// ---------------------------------------------------------------------------
// plan: dry-run a layout over the staged files
// ---------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct PlanRequest {
    #[serde(flatten)]
    pub spec: LayoutSpec,
    /// Grouping shape: "model" pools everything into one model's variants,
    /// "bundle" (default) splits member models by model-name capture.
    #[serde(default)]
    pub target: CarveTarget,
}

/// Everything the layout panel shows — coverage, the grouped tree, per-file
/// highlight spans and resolution chips — comes from this one dry run. Commit
/// executes the same `analyze`, so the preview is the result.
async fn plan(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(request): Json<PlanRequest>,
) -> Result<Json<Plan>, ApiError> {
    let staged = fetch_import(&state, id).await?;
    user.require_can_edit(staged.created_by)?;
    let files = plan_files(&state.db, id).await?;
    let vocab = variant_vocab(&state.db).await?;
    Ok(Json(layout::analyze(
        &request.spec,
        request.target,
        &files,
        &vocab,
    )?))
}

async fn plan_files(
    db: impl sqlx::PgExecutor<'_>,
    import_id: Uuid,
) -> Result<Vec<layout::PlanFile>, ApiError> {
    let rows = sqlx::query!(
        "SELECT id, path, filename FROM files WHERE import_id = $1 ORDER BY path, filename",
        import_id,
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| layout::PlanFile {
            id: r.id,
            path: r.path,
            filename: r.filename,
        })
        .collect())
}

/// The variant-tag vocabulary, lowercased: a raw capture equal to an existing
/// tag name resolves without an explicit value-map entry.
async fn variant_vocab(db: impl sqlx::PgExecutor<'_>) -> Result<HashSet<String>, ApiError> {
    Ok(
        sqlx::query_scalar!(r#"SELECT lower(name::text) as "name!" FROM variant_tags"#)
            .fetch_all(db)
            .await?
            .into_iter()
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// commit: the one decision point — what is this archive?
// ---------------------------------------------------------------------------

/// What the drop was, told once. A box set is bought once, from one creator,
/// under one licence, on one order — so the facts are typed once on the import
/// page and land on *every* model the commit creates, all twenty members of the
/// bundle included. Anything left blank stays blank; nothing here overwrites a
/// value the carve worked out for itself (the tags a layout captured, the
/// creator a bundle already had).
///
/// Flattened into the commit body, so the fields sit at the top level of the
/// JSON exactly as `creator_id` always did.
#[derive(Clone, Default, Deserialize, ToSchema)]
pub struct ImportMeta {
    pub creator_id: Option<Uuid>,
    pub source_url: Option<String>,
    pub license: Option<String>,
    pub purchase_price: Option<f64>,
    pub purchase_date: Option<NaiveDate>,
    pub order_ref: Option<String>,
    /// Model tags (what a model *is*) — added, never replacing what a carve found.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Becomes revision 1 of each model's description.
    pub description_md: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum CommitInput {
    /// One model. Files land in the model's "unsorted" bucket, to be sorted
    /// into variants on the model page — or carved into variants by `layout`.
    NewModel {
        name: Option<String>,
        #[serde(flatten)]
        meta: ImportMeta,
        #[serde(default)]
        layout: Option<LayoutSpec>,
    },
    /// A collection. Files land in the new bundle's "unsorted" bucket, to be
    /// carved into member models on the bundle page — or by `layout`. The
    /// metadata lands on the bundle *and* on every member the carve creates.
    NewBundle {
        name: Option<String>,
        kind: Option<String>,
        #[serde(flatten)]
        meta: ImportMeta,
        #[serde(default)]
        layout: Option<LayoutSpec>,
    },
    /// More files for a bundle that already exists (e.g. the 75mm pack joining
    /// the 32mm one). A `layout` carve reuses matching member models, which is
    /// how the 75mm files land on the models the 32mm drop created.
    Bundle {
        bundle_id: Uuid,
        #[serde(flatten)]
        meta: ImportMeta,
        #[serde(default)]
        layout: Option<LayoutSpec>,
    },
}

#[derive(Serialize, ToSchema)]
pub struct CommitResult {
    /// "model" or "bundle" — where to navigate next.
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Uuid,
    pub slug: String,
}

async fn commit(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<CommitInput>,
) -> Result<Json<CommitResult>, ApiError> {
    let staged = fetch_import(&state, id).await?;
    user.require_can_edit(staged.created_by)?;

    // Committing mid-unpack would strand the files still to be extracted on a
    // deleted import, so wait for the archive to finish.
    if staged.unpacking {
        return Err(ApiError::Conflict(
            "still unpacking — try again when the import finishes".into(),
        ));
    }
    if staged.file_count == 0 {
        return Err(ApiError::BadRequest("nothing staged to import".into()));
    }

    let named = |name: &Option<String>| -> String {
        name.as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&staged.name)
            .to_string()
    };

    let mut tx = state.db.begin().await?;

    // What the import was unpacked *from*. Read while the files still carry
    // `import_id`; the rows themselves are dropped further down, once the commit
    // knows which model or bundle to hang the provenance off.
    let archives = gc::redundant_archives(&mut tx, id).await?;

    // Dry-run the layout carve first: a bad pattern or an unmapped value must
    // fail the commit before anything is created. Same `analyze` as the plan
    // endpoint, so the preview the user confirmed is exactly what happens.
    let (carve_target, layout_spec) = match &input {
        CommitInput::NewModel { layout, .. } => (CarveTarget::Model, layout),
        CommitInput::NewBundle { layout, .. } | CommitInput::Bundle { layout, .. } => {
            (CarveTarget::Bundle, layout)
        }
    };
    // The folders a carve has already read. Collected *before* anything moves,
    // because the carve matches on `path` — flattening first would pull the tree
    // out from under the pattern that is reading it — and because a file on its
    // way to a variant no longer has an `import_id` to find it by afterwards.
    let flatten_ids: Vec<Uuid> = if layout_spec.as_ref().is_some_and(|spec| spec.flatten) {
        sqlx::query_scalar!("SELECT id FROM files WHERE import_id = $1", id)
            .fetch_all(&mut *tx)
            .await?
    } else {
        Vec::new()
    };

    let carve = match layout_spec {
        Some(spec) => {
            let files = plan_files(&mut *tx, id).await?;
            let vocab = variant_vocab(&mut *tx).await?;
            let plan = layout::analyze(spec, carve_target, &files, &vocab)?;
            let unmapped = plan.unmapped_values();
            if !unmapped.is_empty() {
                return Err(ApiError::BadRequest(format!(
                    "unmapped variant tag values: {} — map them (or ignore their group) first",
                    unmapped.join(", ")
                )));
            }
            Some(plan)
        }
        None => None,
    };

    // Models whose browse thumbnail should render once the commit lands.
    let mut render_models: Vec<Uuid> = Vec::new();

    let result = match &input {
        CommitInput::NewModel { name, meta, .. } => {
            let name = named(name);
            let slug = models::unique_slug(&state, &name).await?;
            let model_id: Uuid = sqlx::query_scalar!(
                "INSERT INTO models (name, slug, creator_id, created_by)
                 VALUES ($1, $2, $3, $4) RETURNING id",
                name,
                slug,
                meta.creator_id,
                user.id,
            )
            .fetch_one(&mut *tx)
            .await?;
            apply_meta(&mut tx, &state, model_id, meta, user.id).await?;
            if let Some(plan) = &carve
                && let Some(planned) = plan.models.first()
            {
                add_model_tags(&mut tx, &state, model_id, &planned.tags).await?;
                carve_variants(&mut tx, model_id, &planned.variants, user.id).await?;
            }
            // Whatever the carve didn't claim (or all of it, with no layout)
            // lands in the model's unsorted bucket.
            sqlx::query!(
                "UPDATE files SET model_id = $2, import_id = NULL WHERE import_id = $1",
                id,
                model_id,
            )
            .execute(&mut *tx)
            .await?;
            render_models.push(model_id);
            CommitResult {
                kind: "model".into(),
                id: model_id,
                slug,
            }
        }
        CommitInput::NewBundle {
            name, kind, meta, ..
        } => {
            let name = named(name);
            let bundle_kind = parse_bundle_kind(kind.as_deref())?;
            let slug = bundles::unique_slug(&state, &name).await?;
            let bundle_id: Uuid = sqlx::query_scalar!(
                "INSERT INTO bundles (name, slug, creator_id, source_url, kind, created_by)
                 VALUES ($1, $2, $3, $4, $5::bundle_kind, $6) RETURNING id",
                name,
                slug,
                meta.creator_id,
                meta.source_url,
                bundle_kind as _,
                user.id,
            )
            .fetch_one(&mut *tx)
            .await?;
            if let Some(plan) = &carve {
                let created =
                    carve_into_bundle(&mut tx, &state, bundle_id, meta.creator_id, plan, user.id)
                        .await?;
                // The box set was bought once: what was typed on the import page
                // is true of every model the carve just pulled out of it.
                for model_id in &created {
                    apply_meta(&mut tx, &state, *model_id, meta, user.id).await?;
                }
                render_models.extend(created);
            }
            sqlx::query!(
                "UPDATE files SET bundle_id = $2, import_id = NULL WHERE import_id = $1",
                id,
                bundle_id,
            )
            .execute(&mut *tx)
            .await?;
            CommitResult {
                kind: "bundle".into(),
                id: bundle_id,
                slug,
            }
        }
        CommitInput::Bundle {
            bundle_id, meta, ..
        } => {
            let target = sqlx::query!(
                "SELECT created_by, slug, creator_id FROM bundles WHERE id = $1",
                bundle_id,
            )
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| ApiError::BadRequest("no such bundle".into()))?;
            user.require_can_edit(target.created_by)?;
            if let Some(plan) = &carve {
                let creator = meta.creator_id.or(target.creator_id);
                let created =
                    carve_into_bundle(&mut tx, &state, *bundle_id, creator, plan, user.id).await?;
                // Only the models this drop *created*: a member that was already
                // in the bundle has its own metadata, and a later 75mm pack has no
                // business rewriting it.
                for model_id in &created {
                    apply_meta(&mut tx, &state, *model_id, meta, user.id).await?;
                }
                render_models.extend(created);
            }
            sqlx::query!(
                "UPDATE files SET bundle_id = $2, import_id = NULL WHERE import_id = $1",
                id,
                bundle_id,
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query!(
                "UPDATE bundles SET updated_at = now() WHERE id = $1",
                bundle_id,
            )
            .execute(&mut *tx)
            .await?;
            CommitResult {
                kind: "bundle".into(),
                id: *bundle_id,
                slug: target.slug,
            }
        }
    };

    // Now that the carve has read the folders, throw them away if asked: the
    // variant says what `32mm/supported/` said, and saying it twice only buries
    // the files one level deeper in the model.
    if !flatten_ids.is_empty() {
        sqlx::query!(
            "UPDATE files SET path = '' WHERE id = ANY($1::uuid[])",
            &flatten_ids[..],
        )
        .execute(&mut *tx)
        .await?;
    }

    // Drop the original archive: every byte in it is now also on disk as the
    // files it unpacked into, so keeping it charges the store ~1.3-1.5x forever
    // for a copy nobody browses. What survives is the provenance — name, hash,
    // size of what was dropped — which is the part anyone actually asks for.
    let (owner_model, owner_bundle) = match result.kind.as_str() {
        "model" => (Some(result.id), None),
        _ => (None, Some(result.id)),
    };
    for archive in &archives {
        sqlx::query!(
            "INSERT INTO source_archives (model_id, bundle_id, filename, sha256, size)
             VALUES ($1, $2, $3, $4, $5)",
            owner_model,
            owner_bundle,
            archive.filename,
            archive.sha256,
            archive.size,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("DELETE FROM files WHERE id = $1", archive.file_id)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query!("DELETE FROM imports WHERE id = $1", id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    // Only now that the last reference is committed can the bytes go: a rollback
    // above must not find its blob already deleted from under it.
    let mut freed: i64 = 0;
    for archive in &archives {
        freed += gc::collect_blob(&state, &archive.sha256).await?;
    }
    if freed > 0 {
        tracing::info!(import = %id, freed, archives = archives.len(), "dropped source archives");
    }

    // Every variant gets its own picture: they are all pictures of the model, and
    // a gallery with one thumbnail for a model that ships in four scales is a
    // gallery that hides three quarters of what you bought. One render each, from
    // the STL with the shortest filename — `knight.stl` is the knight,
    // `knight_base_v2_hollow.stl` is a detail of it. A model with no variants
    // (nothing carved) renders once from its unsorted files, on the same rule.
    // (Bundle unsorted files don't render: they are a staging bucket.)
    for model_id in render_models {
        let mut stls = sqlx::query_scalar!(
            r#"SELECT DISTINCT ON (f.variant_id) f.id
               FROM files f
               JOIN model_variants v ON v.id = f.variant_id
               WHERE v.model_id = $1 AND f.filename ILIKE '%.stl'
               ORDER BY f.variant_id, length(f.filename), f.filename"#,
            model_id,
        )
        .fetch_all(&state.db)
        .await?;

        if stls.is_empty() {
            stls = sqlx::query_scalar!(
                r#"SELECT f.id FROM files f
                   WHERE f.model_id = $1 AND f.filename ILIKE '%.stl'
                   ORDER BY length(f.filename), f.filename LIMIT 1"#,
                model_id,
            )
            .fetch_all(&state.db)
            .await?;
        }

        for file_id in stls {
            crate::services::jobs::enqueue(
                &state.db,
                "render_preview",
                serde_json::json!({ "file_id": file_id, "mode": "add" }),
            )
            .await?;
        }
    }

    tracing::info!(import = %id, into = %result.kind, id = %result.id, "import committed");
    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// the carve: execute a plan inside the commit transaction
// ---------------------------------------------------------------------------

/// Assign carved files: each non-empty tag set get-or-creates that variant of
/// the model (the existing merge-by-tag-set semantics), an empty set is the
/// model's unsorted bucket.
async fn carve_variants(
    tx: &mut sqlx::PgConnection,
    model_id: Uuid,
    variants: &[PlanVariant],
    user_id: Uuid,
) -> Result<(), ApiError> {
    for planned in variants {
        if planned.tags.is_empty() {
            sqlx::query!(
                "UPDATE files SET model_id = $1, import_id = NULL WHERE id = ANY($2::uuid[])",
                model_id,
                &planned.files[..],
            )
            .execute(&mut *tx)
            .await?;
            continue;
        }
        let mut tag_ids: Vec<Uuid> = Vec::new();
        for name in &planned.tags {
            let tag_id = upsert_variant_tag(&mut *tx, name).await?;
            if !tag_ids.contains(&tag_id) {
                tag_ids.push(tag_id);
            }
        }
        let variant_id = match variant_with_tag_set(&mut *tx, model_id, &tag_ids).await? {
            Some(existing) => existing,
            None => {
                let new_id: Uuid = sqlx::query_scalar!(
                    "INSERT INTO model_variants (model_id, created_by) VALUES ($1, $2) RETURNING id",
                    model_id,
                    user_id,
                )
                .fetch_one(&mut *tx)
                .await?;
                set_variant_tags(&mut *tx, new_id, &tag_ids).await?;
                new_id
            }
        };
        sqlx::query!(
            "UPDATE files SET variant_id = $1, import_id = NULL WHERE id = ANY($2::uuid[])",
            variant_id,
            &planned.files[..],
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Stamp the import's metadata onto one model. `coalesce` throughout: a blank
/// field on the import page means "nothing to say", not "erase what the carve
/// worked out" — so a member model keeps the creator its bundle gave it unless
/// the import names a different one.
async fn apply_meta(
    tx: &mut sqlx::PgConnection,
    state: &AppState,
    model_id: Uuid,
    meta: &ImportMeta,
    user_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query!(
        r#"UPDATE models SET
             creator_id     = coalesce($2, creator_id),
             source_url     = coalesce($3, source_url),
             license        = coalesce($4, license),
             purchase_price = coalesce($5::float8::numeric(10,2), purchase_price),
             purchase_date  = coalesce($6, purchase_date),
             order_ref      = coalesce($7, order_ref)
           WHERE id = $1"#,
        model_id,
        meta.creator_id,
        meta.source_url,
        meta.license,
        meta.purchase_price,
        meta.purchase_date,
        meta.order_ref,
    )
    .execute(&mut *tx)
    .await?;

    // Additive, like every other tagging path: the layout's captured tags and
    // the ones typed on the import page both describe the model.
    add_model_tags(tx, state, model_id, &meta.tags).await?;

    if let Some(body) = meta
        .description_md
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        sqlx::query!(
            "INSERT INTO model_description_revisions (model_id, body_md, created_by)
             VALUES ($1, $2, $3)",
            model_id,
            body,
            user_id,
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Additive model tagging — a carve never removes tags a model already has.
async fn add_model_tags(
    tx: &mut sqlx::PgConnection,
    state: &AppState,
    model_id: Uuid,
    tags: &[String],
) -> Result<(), ApiError> {
    for name in tags {
        let tag = upsert_tag(state, name).await?;
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

/// Carve a bundle-target plan: each planned (name + model-tag set) reuses a
/// member model whose name matches and whose tags cover the group's, else
/// creates one — which is how a later DownloadAll_75mm drop lands its files on
/// the models the 32mm drop created. Returns newly created model ids.
async fn carve_into_bundle(
    tx: &mut sqlx::PgConnection,
    state: &AppState,
    bundle_id: Uuid,
    bundle_creator: Option<Uuid>,
    plan: &Plan,
    user_id: Uuid,
) -> Result<Vec<Uuid>, ApiError> {
    let members = sqlx::query!(
        r#"SELECT m.id, m.name,
                  coalesce((SELECT array_agg(lower(t.name::text)) FROM model_tags mt
                            JOIN tags t ON t.id = mt.tag_id WHERE mt.model_id = m.id), '{}')
                      as "tags!: Vec<String>"
           FROM models m JOIN bundle_models bm ON bm.model_id = m.id
           WHERE bm.bundle_id = $1"#,
        bundle_id,
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut reserved_slugs: HashSet<String> = HashSet::new();
    let mut created = Vec::new();
    for planned in &plan.models {
        let wanted: Vec<String> = planned.tags.iter().map(|t| t.to_lowercase()).collect();
        let existing = members.iter().find(|m| {
            m.name.to_lowercase() == planned.name.to_lowercase()
                && wanted.iter().all(|t| m.tags.contains(t))
        });
        let model_id = match existing {
            Some(member) => member.id,
            None => {
                let slug = unique_member_slug(&mut *tx, &planned.name, &mut reserved_slugs).await?;
                let model_id: Uuid = sqlx::query_scalar!(
                    "INSERT INTO models (name, slug, creator_id, created_by)
                     VALUES ($1, $2, $3, $4) RETURNING id",
                    planned.name,
                    slug,
                    bundle_creator,
                    user_id,
                )
                .fetch_one(&mut *tx)
                .await?;
                sqlx::query!(
                    "INSERT INTO bundle_models (bundle_id, model_id) VALUES ($1, $2)",
                    bundle_id,
                    model_id,
                )
                .execute(&mut *tx)
                .await?;
                created.push(model_id);
                model_id
            }
        };
        add_model_tags(&mut *tx, state, model_id, &planned.tags).await?;
        carve_variants(&mut *tx, model_id, &planned.variants, user_id).await?;
    }
    Ok(created)
}

/// Like `models::unique_slug`, but reading through the commit transaction and
/// aware of slugs this same carve has claimed but not yet made visible.
async fn unique_member_slug(
    tx: &mut sqlx::PgConnection,
    name: &str,
    reserved: &mut HashSet<String>,
) -> Result<String, ApiError> {
    let base = slugify(name);
    let taken: Vec<String> = sqlx::query_scalar!(
        "SELECT slug FROM models WHERE slug = $1 OR slug LIKE $1 || '-%'",
        base,
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut candidate = base.clone();
    let mut n = 2;
    while taken.contains(&candidate) || reserved.contains(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    reserved.insert(candidate.clone());
    Ok(candidate)
}
