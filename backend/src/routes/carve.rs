//! Carving a model that already exists: run an import layout over the files a
//! model is *already* holding and re-shape them in place.
//!
//! The import page carves an archive on the way in, and gets one chance to do
//! it. What arrives as one model is often several — a "Dragon" whose folders
//! turn out to be a head, a body and three bases — and the mistake only shows
//! up once you are looking at the model. Re-importing to fix that means finding
//! the archive again; carving fixes it from the library.
//!
//! The same [`layout`] machinery drives it, under [`CarveTarget::Carve`], which
//! is what makes the two halves of the operation one thing:
//!
//! * a file whose captures name **no model** stays on this model, sorted into
//!   the variant its variant tags describe — a plain variant split;
//! * a file that **does** capture a model name was never this model, and splits
//!   out into a model of its own.
//!
//! Anything the rules don't match is left exactly where it is: a carve claims
//! what it recognises and nothing else.
//!
//! Split models are pieces of one purchase, so they don't just scatter: the
//! carve puts every model it produced — the original included — into a new
//! bundle, and if the original already belonged to a bundle the new models join
//! that one too. That is why the page redirects to the bundle when anything
//! split, and back to the model when nothing did.

use std::collections::HashSet;

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::custom_fields::{self, ValueOwner};
use crate::routes::files::{FileKind, FileRecord};
use crate::routes::imports::{
    TagMaps, Untagged, add_model_tags, carve_variants, disambiguate_filenames, unique_member_slug,
    upsert_tags_bulk, upsert_variant_tags_bulk, variant_vocab,
};
use crate::routes::{bundles, models};
use crate::services::layout::{self, CarveTarget, LayoutSpec, Plan};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/models/{id}/carve/files", get(carve_files))
        .route("/api/models/{id}/carve/plan", post(plan))
        .route("/api/models/{id}/carve", post(carve))
}

/// Everything a carve can touch: the model's unsorted bucket plus every file of
/// every variant, as one flat list in path order — which is the shape both the
/// layout and the annotated file list want.
///
/// Archives are left out for the same reason the import plan leaves them out:
/// a `.zip` kept for provenance is not a part of the model to be sorted, and
/// matching a layout against its filename can only drag the coverage count down
/// against a file that was never a candidate.
async fn model_carve_files(
    db: impl sqlx::PgExecutor<'_>,
    model_id: Uuid,
) -> Result<Vec<FileRow>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT f.id, f.blob_sha256, f.path, f.filename, f.mime,
                  f.kind as "kind: FileKind", f.created_at, b.size
           FROM files f
           JOIN blobs b ON b.sha256 = f.blob_sha256
           LEFT JOIN model_variants v ON v.id = f.variant_id
           WHERE (f.model_id = $1 OR v.model_id = $1)
             AND f.kind <> 'archive'::file_kind
           ORDER BY f.path, f.filename"#,
        model_id,
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| FileRow {
            record: FileRecord {
                id: r.id,
                blob_sha256: r.blob_sha256,
                path: r.path.clone(),
                filename: r.filename.clone(),
                mime: r.mime,
                kind: r.kind,
                size: r.size,
                created_at: r.created_at,
                unpack: None,
            },
            plan: layout::PlanFile {
                id: r.id,
                path: r.path,
                filename: r.filename,
            },
        })
        .collect())
}

/// One carveable file, in both the shapes this module needs: what the page
/// lists, and what the layout matches on.
struct FileRow {
    record: FileRecord,
    plan: layout::PlanFile,
}

async fn carve_files(
    State(state): State<AppState>,
    _user: User,
    Path(key): Path<String>,
) -> Result<Json<Vec<FileRecord>>, ApiError> {
    let id = models::resolve_id(&state, &key).await?;
    let files = model_carve_files(&state.db, id).await?;
    Ok(Json(files.into_iter().map(|f| f.record).collect()))
}

/// The model as the carve needs it: who may edit it, and the facts a split-out
/// piece inherits (it came out of the same box, from the same creator, on the
/// same order).
struct Subject {
    id: Uuid,
    name: String,
    slug: String,
    created_by: Uuid,
    creator_id: Option<Uuid>,
    creator_ref: Option<String>,
    model_version: Option<String>,
}

async fn fetch_subject(state: &AppState, key: &str) -> Result<Subject, ApiError> {
    let id = models::resolve_id(state, key).await?;
    let row = sqlx::query!(
        "SELECT name, slug, created_by, creator_id, creator_ref, model_version
         FROM models WHERE id = $1",
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Subject {
        id,
        name: row.name,
        slug: row.slug,
        created_by: row.created_by,
        creator_id: row.creator_id,
        creator_ref: row.creator_ref,
        model_version: row.model_version,
    })
}

// ---------------------------------------------------------------------------
// plan: dry-run a layout over the model's own files
// ---------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct PlanRequest {
    #[serde(flatten)]
    pub spec: LayoutSpec,
    /// As on the import plan: the tallies without the per-file annotations, for
    /// the picker that dry-runs every saved layout just to rank them.
    #[serde(default)]
    pub counts_only: bool,
}

/// Preview a carve. The same `analyze` the carve itself runs, so what the page
/// shows is what happens.
async fn plan(
    State(state): State<AppState>,
    user: User,
    Path(key): Path<String>,
    Json(request): Json<PlanRequest>,
) -> Result<Json<Plan>, ApiError> {
    let subject = fetch_subject(&state, &key).await?;
    user.require_can_edit(subject.created_by)?;
    let files = model_carve_files(&state.db, subject.id).await?;
    let vocab = variant_vocab(&state.db).await?;
    let plan_files: Vec<layout::PlanFile> = files.into_iter().map(|f| f.plan).collect();
    let mut plan = layout::analyze(&request.spec, CarveTarget::Carve, &plan_files, &vocab)?;
    if request.counts_only {
        plan.annotations = Vec::new();
        plan.models = Vec::new();
        plan.model_names = Vec::new();
    }
    Ok(Json(plan))
}

// ---------------------------------------------------------------------------
// carve: execute it
// ---------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct CarveInput {
    #[serde(flatten)]
    pub spec: LayoutSpec,
    /// What to call the bundle the split models are gathered into. Blank (or
    /// absent) takes the model's own name: the pieces of a "Dragon" are still
    /// the Dragon. Ignored when nothing splits out — there is no bundle then.
    #[serde(default)]
    pub bundle_name: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct CarveResult {
    /// Where to go next: `"bundle"` when models split out (the bundle holds all
    /// of them, including the one that was carved), `"model"` when the carve
    /// only re-sorted this model's own variants.
    pub kind: String,
    pub id: Uuid,
    pub slug: String,
    /// Models split out of this one.
    pub models_created: usize,
    /// Variants the carve left holding nothing, and so removed.
    pub variants_removed: usize,
}

async fn carve(
    State(state): State<AppState>,
    user: User,
    Path(key): Path<String>,
    Json(input): Json<CarveInput>,
) -> Result<Json<CarveResult>, ApiError> {
    let subject = fetch_subject(&state, &key).await?;
    user.require_can_edit(subject.created_by)?;

    let mut tx = state.db.begin().await?;

    // Dry-run first, inside the transaction: a bad pattern or an unmapped value
    // must fail before anything moves.
    let files = model_carve_files(&mut *tx, subject.id).await?;
    let vocab = variant_vocab(&mut *tx).await?;
    let plan_files: Vec<layout::PlanFile> = files.into_iter().map(|f| f.plan).collect();
    let plan = layout::analyze(&input.spec, CarveTarget::Carve, &plan_files, &vocab)?;
    let unmapped = plan.unmapped_values();
    if !unmapped.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "unmapped variant tag values: {} — map them (or ignore their group) first",
            unmapped.join(", ")
        )));
    }
    if plan.carved == 0 {
        return Err(ApiError::BadRequest(
            "no file matches this layout — there is nothing to carve".into(),
        ));
    }

    // Resolve every tag name up front, one bulk upsert per vocabulary, so the
    // per-model work below is map lookups (see the import commit).
    let mut vtag_names: Vec<String> = Vec::new();
    let mut mtag_names: Vec<String> = plan.model_tag_order.clone();
    for planned in &plan.models {
        mtag_names.extend(planned.tags.iter().cloned());
        for variant in &planned.variants {
            vtag_names.extend(variant.tags.iter().cloned());
        }
    }
    let tags = TagMaps {
        variant: upsert_variant_tags_bulk(&mut tx, &vtag_names).await?,
        model: upsert_tags_bulk(&mut tx, &mtag_names).await?,
    };

    // The unnamed planned model is this model's own share of the carve; every
    // named one is a piece that leaves.
    let home = plan.models.iter().find(|m| m.name.is_empty());
    let splits: Vec<&layout::PlanModel> = plan
        .models
        .iter()
        .filter(|m| !m.name.is_empty())
        .collect::<Vec<_>>();

    if let Some(home) = home {
        add_model_tags(&mut tx, subject.id, &home.tags, &tags.model).await?;
        // A creator id or version the layout read off this model's own files is
        // about this model — but only fills a blank, since what is already on
        // the model was put there deliberately.
        if subject.creator_ref.is_none() || subject.model_version.is_none() {
            sqlx::query!(
                "UPDATE models
                 SET creator_ref = coalesce(creator_ref, $2),
                     model_version = coalesce(model_version, $3)
                 WHERE id = $1",
                subject.id,
                home.creator_ref,
                home.model_version,
            )
            .execute(&mut *tx)
            .await?;
        }
        // Untagged matched files land in the anonymous variant, not loose in the
        // unsorted bucket: a carve's whole output is variants, and the anonymous
        // variant *is* the model's plain bucket of files, as a first-class
        // sibling of the tagged ones rather than as leftovers.
        carve_variants(
            &mut tx,
            subject.id,
            &home.variants,
            user.id,
            Untagged::AnonymousVariant,
            &tags.variant,
            false,
        )
        .await?;
    }

    // The custom-field values the split models inherit — read once, before any
    // of them exist, since they all inherit the same thing.
    let inherited_values = if splits.is_empty() {
        Vec::new()
    } else {
        custom_fields::values_of(&mut tx, ValueOwner::Model(subject.id)).await?
    };

    let mut reserved_slugs: HashSet<String> = HashSet::new();
    let mut created: Vec<Uuid> = Vec::new();
    for planned in &splits {
        let slug = unique_member_slug(&mut tx, &planned.name, &mut reserved_slugs).await?;
        // Everything about *where this came from* carries over: it was bought
        // once, from one creator, on one order, and splitting it up doesn't
        // change any of that. Only the creator's own id and version come from
        // the layout, since those identify the piece rather than the purchase.
        let model_id: Uuid = sqlx::query_scalar!(
            "INSERT INTO models
                 (name, slug, creator_id, creator_ref, model_version, source_url, license,
                  purchase_price, purchase_date, order_ref, created_by)
             SELECT $1, $2, m.creator_id, $3, $4, m.source_url, m.license,
                    m.purchase_price, m.purchase_date, m.order_ref, $5
             FROM models m WHERE m.id = $6
             RETURNING id",
            planned.name,
            slug,
            planned.creator_ref,
            planned.model_version,
            user.id,
            subject.id,
        )
        .fetch_one(&mut *tx)
        .await?;
        // What the model *is* survives the split — a piece of a 28mm fantasy
        // dragon is still 28mm fantasy — on top of whatever the layout captured.
        sqlx::query!(
            "INSERT INTO model_tags (model_id, tag_id)
             SELECT $1, tag_id FROM model_tags WHERE model_id = $2
             ON CONFLICT DO NOTHING",
            model_id,
            subject.id,
        )
        .execute(&mut *tx)
        .await?;
        add_model_tags(&mut tx, model_id, &planned.tags, &tags.model).await?;
        custom_fields::copy_values_onto(
            &mut tx,
            &inherited_values,
            ValueOwner::Model(model_id),
            |_| true,
            &user,
        )
        .await?;
        carve_variants(
            &mut tx,
            model_id,
            &planned.variants,
            user.id,
            Untagged::AnonymousVariant,
            &tags.variant,
            true,
        )
        .await?;
        created.push(model_id);
    }

    // Now that the carve has read the folders, throw them away if asked — and
    // only for the files it claimed, so an unmatched file keeps the tree that a
    // second pass will have to match on. A `folder` capture is the general case
    // of the same rewrite, and is applied second so it wins where it fired.
    let claimed: Vec<Uuid> = plan
        .annotations
        .iter()
        .filter(|a| a.matched)
        .map(|a| a.id)
        .collect();
    if input.spec.flatten && !claimed.is_empty() {
        sqlx::query!(
            "UPDATE files SET path = '' WHERE id = ANY($1::uuid[])",
            &claimed,
        )
        .execute(&mut *tx)
        .await?;
    }
    for (folder, ids) in plan.folder_moves(Some(&claimed)) {
        sqlx::query!(
            "UPDATE files SET path = $2 WHERE id = ANY($1::uuid[])",
            &ids[..],
            folder,
        )
        .execute(&mut *tx)
        .await?;
    }

    // A variant the carve emptied is not a variant any more, just a label with
    // nothing behind it. Only ones with nothing else to lose go: a name, print
    // notes or a picture is content someone put there, and a carve is not
    // licence to delete it.
    let variants_removed = sqlx::query!(
        "DELETE FROM model_variants v
         WHERE v.model_id = $1 AND v.name IS NULL AND v.print_notes IS NULL
           AND NOT EXISTS (SELECT 1 FROM files f WHERE f.variant_id = v.id)
           AND NOT EXISTS (SELECT 1 FROM images i WHERE i.variant_id = v.id)",
        subject.id,
    )
    .execute(&mut *tx)
    .await?
    .rows_affected() as usize;

    // Split models are pieces of one thing, so they are kept together as one:
    // a bundle over every model this carve produced, the original included.
    let mut result = CarveResult {
        kind: "model".into(),
        id: subject.id,
        slug: subject.slug.clone(),
        models_created: created.len(),
        variants_removed,
    };
    if !created.is_empty() {
        // Wherever the original already belonged, its pieces belong too —
        // otherwise carving a bundle's member quietly drops most of it out of
        // that bundle. Done before the new bundle is made, so the new one isn't
        // in the list.
        sqlx::query!(
            "INSERT INTO bundle_models (bundle_id, model_id)
             SELECT bm.bundle_id, m FROM bundle_models bm, unnest($2::uuid[]) AS m
             WHERE bm.model_id = $1
             ON CONFLICT DO NOTHING",
            subject.id,
            &created,
        )
        .execute(&mut *tx)
        .await?;

        let bundle_name = input
            .bundle_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&subject.name)
            .to_string();
        let bundle_slug = bundles::unique_slug(&state, &bundle_name, None, None).await?;
        // Owned by whoever owns the model it came out of, not by whoever pressed
        // the button — the same rule a bundle split follows.
        let bundle_id: Uuid = sqlx::query_scalar!(
            "INSERT INTO bundles (name, slug, creator_id, created_by)
             VALUES ($1, $2, $3, $4) RETURNING id",
            bundle_name,
            bundle_slug,
            subject.creator_id,
            subject.created_by,
        )
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query!(
            "INSERT INTO bundle_models (bundle_id, model_id)
             SELECT $1, m FROM unnest($2::uuid[]) AS m",
            bundle_id,
            &created
                .iter()
                .copied()
                .chain(std::iter::once(subject.id))
                .collect::<Vec<_>>(),
        )
        .execute(&mut *tx)
        .await?;
        result = CarveResult {
            kind: "bundle".into(),
            id: bundle_id,
            slug: bundle_slug,
            ..result
        };
    }

    // Both path rewrites above are blind — a flatten sends every claimed file to
    // `''` — so two files told apart only by their folder can now agree on
    // (owner, path, filename). Settle that last, over every model the carve
    // touched, once nothing else is moving.
    let touched: Vec<Uuid> = created
        .iter()
        .copied()
        .chain(std::iter::once(subject.id))
        .collect();
    disambiguate_filenames(&mut tx, &touched, None).await?;

    sqlx::query!(
        "UPDATE models SET updated_at = now() WHERE id = ANY($1::uuid[])",
        &touched,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // A split model has no picture of its own yet — the original's images stayed
    // with the original. Render one per variant, from the STL with the shortest
    // filename, exactly as the import commit does.
    for model_id in &created {
        enqueue_previews(&state, *model_id).await?;
    }

    tracing::info!(
        model = %subject.id,
        created = created.len(),
        variants_removed,
        into = %result.kind,
        "carved a model",
    );
    Ok(Json(result))
}

/// Queue a preview render for every variant of a freshly split model, from the
/// STL with the shortest filename — `knight.stl` is the knight,
/// `knight_base_v2_hollow.stl` is a detail of it.
async fn enqueue_previews(state: &AppState, model_id: Uuid) -> Result<(), ApiError> {
    let stls = sqlx::query_scalar!(
        r#"SELECT DISTINCT ON (f.variant_id) f.id
           FROM files f
           JOIN model_variants v ON v.id = f.variant_id
           WHERE v.model_id = $1 AND f.filename ILIKE '%.stl'
           ORDER BY f.variant_id, length(f.filename), f.filename"#,
        model_id,
    )
    .fetch_all(&state.db)
    .await?;
    for file_id in stls {
        crate::services::jobs::enqueue(
            &state.db,
            "render_preview",
            serde_json::json!({ "file_id": file_id, "mode": "add" }),
        )
        .await?;
    }
    Ok(())
}
