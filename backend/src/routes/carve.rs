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
//!
//! # Carving a whole bundle
//!
//! A bundle is usually one purchase that arrived as one tree, so its members
//! share a naming scheme — and a mistake in how it was read is a mistake on
//! every one of them. `/api/bundles/{id}/carve` runs **one** layout over every
//! member: each member is carved exactly as it would be on its own, and because
//! a split model inherits the subject's bundle memberships, the pieces land in
//! the bundle already being carved rather than in a new one per member.
//!
//! The preview is the members' plans merged into one (see `merge_plans`), which
//! is honest about the two ways a bundle-wide plan differs from a single
//! model's: every member's own share reads as one "this model" row, and two
//! members that both capture the name "Head" list as two rows, because that is
//! two new models.

use std::collections::{BTreeMap, HashSet};

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
use crate::services::layout::{self, CarveTarget, LayoutSpec, Plan, PlanModel};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/models/{id}/carve/files", get(carve_files))
        .route("/api/models/{id}/carve/plan", post(plan))
        .route("/api/models/{id}/carve", post(carve))
        .route("/api/bundles/{id}/carve/files", get(bundle_carve_files))
        .route("/api/bundles/{id}/carve/plan", post(plan_bundle))
        .route("/api/bundles/{id}/carve", post(carve_bundle))
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
    let rows = sqlx::query_as!(
        CarveFileRow,
        // `!`: computed columns, and the model id is only NULL-able because the
        // owner may be the variant instead (see the coalesce).
        r#"SELECT f.id, f.blob_sha256, f.path, f.filename, f.mime,
                  f.kind as "kind: FileKind", f.created_at, b.size,
                  coalesce(f.model_id, v.model_id) as "model_id!",
                  (f.variant_id IS NOT NULL) as "in_variant!"
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
    Ok(rows.into_iter().map(FileRow::from).collect())
}

/// Every carveable file of every member of a bundle, in one query — the same
/// per-model list, unioned. Grouped by member before it reaches the layout: the
/// rules match on the path a file has *on its own model*, so a bundle-wide carve
/// is a per-member carve run several times, never one carve over a merged tree.
async fn bundle_member_files(
    db: impl sqlx::PgExecutor<'_>,
    bundle_id: Uuid,
) -> Result<Vec<FileRow>, ApiError> {
    let rows = sqlx::query_as!(
        CarveFileRow,
        r#"SELECT f.id, f.blob_sha256, f.path, f.filename, f.mime,
                  f.kind as "kind: FileKind", f.created_at, b.size,
                  coalesce(f.model_id, v.model_id) as "model_id!",
                  (f.variant_id IS NOT NULL) as "in_variant!"
           FROM files f
           JOIN blobs b ON b.sha256 = f.blob_sha256
           LEFT JOIN model_variants v ON v.id = f.variant_id
           WHERE coalesce(f.model_id, v.model_id) IN
                     (SELECT model_id FROM bundle_models WHERE bundle_id = $1)
             AND f.kind <> 'archive'::file_kind
           ORDER BY f.path, f.filename"#,
        bundle_id,
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(FileRow::from).collect())
}

/// The columns both carve-file queries select — named so the two share one
/// mapping into [`FileRow`] rather than each growing its own copy.
struct CarveFileRow {
    id: Uuid,
    blob_sha256: String,
    path: String,
    filename: String,
    mime: Option<String>,
    kind: FileKind,
    created_at: chrono::DateTime<chrono::Utc>,
    size: i64,
    model_id: Uuid,
    in_variant: bool,
}

/// One carveable file, in both the shapes this module needs: what the page
/// lists, and what the layout matches on.
struct FileRow {
    /// The model holding it — its own, or its variant's.
    model_id: Uuid,
    record: FileRecord,
    plan: layout::PlanFile,
}

impl From<CarveFileRow> for FileRow {
    fn from(r: CarveFileRow) -> Self {
        FileRow {
            model_id: r.model_id,
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
                in_variant: r.in_variant,
            },
        }
    }
}

/// Split the flat list into the per-model lists the layout is run over, keeping
/// each one in the path order the query produced.
fn by_model(files: Vec<FileRow>) -> BTreeMap<Uuid, Vec<layout::PlanFile>> {
    let mut out: BTreeMap<Uuid, Vec<layout::PlanFile>> = BTreeMap::new();
    for file in files {
        out.entry(file.model_id).or_default().push(file.plan);
    }
    out
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

async fn bundle_carve_files(
    State(state): State<AppState>,
    _user: User,
    Path(key): Path<String>,
) -> Result<Json<Vec<FileRecord>>, ApiError> {
    let id = bundles::resolve_id(&state, &key).await?;
    let files = bundle_member_files(&state.db, id).await?;
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

/// Every member of a bundle, as carve subjects in name order — what a
/// bundle-wide carve runs over, and the order its members are carved in.
async fn bundle_subjects(
    db: impl sqlx::PgExecutor<'_>,
    bundle_id: Uuid,
) -> Result<Vec<Subject>, ApiError> {
    Ok(sqlx::query_as!(
        Subject,
        "SELECT m.id, m.name, m.slug, m.created_by, m.creator_id, m.creator_ref, m.model_version
         FROM models m JOIN bundle_models bm ON bm.model_id = m.id
         WHERE bm.bundle_id = $1
         ORDER BY m.name",
        bundle_id,
    )
    .fetch_all(db)
    .await?)
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
    let plan = layout::analyze(&request.spec, CarveTarget::Carve, &plan_files, &vocab)?;
    Ok(Json(strip(plan, request.counts_only)))
}

/// Preview a bundle-wide carve: one plan per member, merged. The members are
/// planned separately for the same reason they are carved separately — the rules
/// match on the path a file has on its own model — and merged so the page can
/// show one preview of what pressing the button does.
async fn plan_bundle(
    State(state): State<AppState>,
    user: User,
    Path(key): Path<String>,
    Json(request): Json<PlanRequest>,
) -> Result<Json<Plan>, ApiError> {
    let bundle_id = bundles::resolve_id(&state, &key).await?;
    user.require_can_edit(bundles::bundle_created_by(&state, bundle_id).await?)?;
    let vocab = variant_vocab(&state.db).await?;
    let files = by_model(bundle_member_files(&state.db, bundle_id).await?);
    let mut plans = Vec::with_capacity(files.len());
    for member_files in files.values() {
        plans.push(layout::analyze(
            &request.spec,
            CarveTarget::Carve,
            member_files,
            &vocab,
        )?);
    }
    Ok(Json(strip(merge_plans(plans), request.counts_only)))
}

/// Drop everything the coverage ranking doesn't read. The annotations are the
/// whole weight of a plan — one entry per file — and the picker dry-runs every
/// saved layout just to compare their match counts.
fn strip(mut plan: Plan, counts_only: bool) -> Plan {
    if counts_only {
        plan.annotations = Vec::new();
        plan.models = Vec::new();
        plan.model_names = Vec::new();
    }
    plan
}

/// Fold the members' plans into the one the page draws.
///
/// Two things are deliberately *not* merged. Every member's own share (the
/// unnamed model) collapses into a single row — thirty members would otherwise
/// list thirty identical "this model" entries — while two members that both
/// capture the name "Head" stay two rows, because the carve really does make two
/// models. Files keep their own annotations either way, so the file list is
/// exactly the union of what each member's rules saw.
fn merge_plans(plans: Vec<Plan>) -> Plan {
    let mut out = Plan {
        total: 0,
        matched: 0,
        carved: 0,
        rules: Vec::new(),
        models: Vec::new(),
        model_names: Vec::new(),
        annotations: Vec::new(),
        members: Vec::new(),
        model_tag_order: Vec::new(),
    };
    let mut home: Option<PlanModel> = None;
    for plan in plans {
        out.total += plan.total;
        out.matched += plan.matched;
        out.carved += plan.carved;
        out.annotations.extend(plan.annotations);
        for name in plan.model_names {
            if !out.model_names.contains(&name) {
                out.model_names.push(name);
            }
        }
        for tag in plan.model_tag_order {
            if !out.model_tag_order.contains(&tag) {
                out.model_tag_order.push(tag);
            }
        }
        for model in plan.models {
            if !model.name.is_empty() {
                out.models.push(model);
            } else if let Some(home) = &mut home {
                merge_home(home, model);
            } else {
                home = Some(model);
            }
        }
        // Every member ran the same spec, so the rule blocks are index-aligned:
        // what differs is which examples and which raw values each member's files
        // happened to show. The editor needs the union — a value only member 12
        // captures still has to appear in the mapping table, or the carve refuses
        // on an unmapped value the page never offered.
        if out.rules.is_empty() {
            out.rules = plan.rules;
            continue;
        }
        for (into, from) in out.rules.iter_mut().zip(plan.rules) {
            for (group, from_group) in into.groups.iter_mut().zip(from.groups) {
                for example in from_group.examples {
                    if group.examples.len() < 3 && !group.examples.contains(&example) {
                        group.examples.push(example);
                    }
                }
            }
            for value in from.values {
                if !into.values.iter().any(|v| v.raw == value.raw) {
                    into.values.push(value);
                }
            }
        }
    }
    // `analyze` emits each rule's values in canonical order; keep that after the
    // merge so the mapping table doesn't reshuffle as the plans arrive.
    for rule in &mut out.rules {
        rule.values.sort_by_key(|v| v.raw.to_lowercase());
    }
    // The members' own share leads the list, as it does on a single model.
    if let Some(home) = home {
        out.models.insert(0, home);
    }
    out
}

/// Add one member's own share to the collapsed "stays on the member it is
/// already on" row, folding variants that resolved the same tag set.
fn merge_home(into: &mut PlanModel, from: PlanModel) {
    into.file_count += from.file_count;
    for tag in from.tags {
        if !into.tags.contains(&tag) {
            into.tags.push(tag);
        }
    }
    for variant in from.variants {
        // `analyze` sorts a variant's tags, so set equality is list equality.
        match into.variants.iter_mut().find(|v| v.tags == variant.tags) {
            Some(existing) => {
                existing.file_count += variant.file_count;
                existing.files.extend(variant.files);
            }
            None => into.variants.push(variant),
        }
    }
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

/// What carving one model did.
#[derive(Default)]
struct CarveOutcome {
    /// Files the layout placed. Zero means the rules recognised nothing on this
    /// model and nothing was written.
    carved: usize,
    /// Models that split out of it.
    created: Vec<Uuid>,
    /// Variants the carve left holding nothing, and so removed.
    variants_removed: usize,
}

/// Carve one model, inside an open transaction: dry-run the layout over its own
/// files, re-sort what stays into variants, split out what named itself, apply
/// the path rewrites, and drop the variants the carve emptied.
///
/// The models it produced are handed back rather than filed anywhere, because
/// where they go is the caller's question: a lone carve gathers them into a new
/// bundle of their own, while a bundle-wide carve already has the bundle they
/// belong in. They do inherit the subject's *existing* bundle memberships here,
/// since that holds either way — carving a member must not drop its pieces out of
/// the bundle it was part of.
async fn carve_one(
    tx: &mut sqlx::PgConnection,
    subject: &Subject,
    spec: &LayoutSpec,
    user: &User,
) -> Result<CarveOutcome, ApiError> {
    // Dry-run first, inside the transaction: a bad pattern or an unmapped value
    // must fail before anything moves.
    let files = model_carve_files(&mut *tx, subject.id).await?;
    let vocab = variant_vocab(&mut *tx).await?;
    let plan_files: Vec<layout::PlanFile> = files.into_iter().map(|f| f.plan).collect();
    let plan = layout::analyze(spec, CarveTarget::Carve, &plan_files, &vocab)?;
    let unmapped = plan.unmapped_values();
    if !unmapped.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "unmapped variant tag values: {} — map them (or ignore their group) first",
            unmapped.join(", ")
        )));
    }
    // Nothing for this layout to do here. Not an error at this level: a
    // bundle-wide carve runs one set of rules over every member, and a member
    // they don't recognise is simply left alone. The lone-model endpoint refuses
    // outright instead — there, this *is* the whole operation.
    if plan.carved == 0 {
        return Ok(CarveOutcome::default());
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
        variant: upsert_variant_tags_bulk(&mut *tx, &vtag_names).await?,
        model: upsert_tags_bulk(&mut *tx, &mtag_names).await?,
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
        add_model_tags(&mut *tx, subject.id, &home.tags, &tags.model).await?;
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
            &mut *tx,
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
        custom_fields::values_of(&mut *tx, ValueOwner::Model(subject.id)).await?
    };

    let mut reserved_slugs: HashSet<String> = HashSet::new();
    let mut created: Vec<Uuid> = Vec::new();
    for planned in &splits {
        let slug = unique_member_slug(&mut *tx, &planned.name, &mut reserved_slugs).await?;
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
        add_model_tags(&mut *tx, model_id, &planned.tags, &tags.model).await?;
        custom_fields::copy_values_onto(
            &mut *tx,
            &inherited_values,
            ValueOwner::Model(model_id),
            |_| true,
            user,
        )
        .await?;
        carve_variants(
            &mut *tx,
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
    if spec.flatten && !claimed.is_empty() {
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

    // Wherever the subject already belonged, its pieces belong too — otherwise
    // carving a bundle's member quietly drops most of it out of that bundle. This
    // is also the whole of a bundle-wide carve's filing: the members are already
    // in the bundle, so the pieces join it here.
    if !created.is_empty() {
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
    disambiguate_filenames(&mut *tx, &touched, None).await?;

    sqlx::query!(
        "UPDATE models SET updated_at = now() WHERE id = ANY($1::uuid[])",
        &touched,
    )
    .execute(&mut *tx)
    .await?;

    Ok(CarveOutcome {
        carved: plan.carved,
        created,
        variants_removed,
    })
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
    let outcome = carve_one(&mut tx, &subject, &input.spec, &user).await?;
    if outcome.carved == 0 {
        return Err(ApiError::BadRequest(
            "no file matches this layout — there is nothing to carve".into(),
        ));
    }

    // Split models are pieces of one thing, so they are kept together as one:
    // a bundle over every model this carve produced, the original included.
    let mut result = CarveResult {
        kind: "model".into(),
        id: subject.id,
        slug: subject.slug.clone(),
        models_created: outcome.created.len(),
        variants_removed: outcome.variants_removed,
    };
    if !outcome.created.is_empty() {
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
            &outcome
                .created
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

    tx.commit().await?;

    // A split model has no picture of its own yet — the original's images stayed
    // with the original. Render one per variant, from the STL with the shortest
    // filename, exactly as the import commit does.
    for model_id in &outcome.created {
        enqueue_previews(&state, *model_id).await?;
    }

    tracing::info!(
        model = %subject.id,
        created = outcome.created.len(),
        variants_removed = outcome.variants_removed,
        into = %result.kind,
        "carved a model",
    );
    Ok(Json(result))
}

#[derive(Serialize, ToSchema)]
pub struct BundleCarveResult {
    /// Members the layout actually moved files on. The rest matched nothing and
    /// were left untouched — not an error: one layout over a whole bundle is
    /// expected to have nothing to say about some of it.
    pub members_carved: usize,
    /// New member models split out, across every member.
    pub models_created: usize,
    /// Variants the carve left holding nothing, and so removed.
    pub variants_removed: usize,
}

/// Carve every member of a bundle with one layout.
///
/// One transaction over the lot: the members share a naming scheme, so a layout
/// that turns out to be wrong is wrong about all of them, and half-carving a
/// bundle would leave a mess with no undo. Everything the members produce joins
/// this bundle (see [`carve_one`]) rather than making a bundle per member.
async fn carve_bundle(
    State(state): State<AppState>,
    user: User,
    Path(key): Path<String>,
    Json(spec): Json<LayoutSpec>,
) -> Result<Json<BundleCarveResult>, ApiError> {
    let bundle_id = bundles::resolve_id(&state, &key).await?;
    user.require_can_edit(bundles::bundle_created_by(&state, bundle_id).await?)?;

    let mut tx = state.db.begin().await?;
    let subjects = bundle_subjects(&mut *tx, bundle_id).await?;
    if subjects.is_empty() {
        return Err(ApiError::BadRequest(
            "this bundle has no member models to carve".into(),
        ));
    }
    // Owning the bundle is not owning its members: an editor who may edit the
    // crate may still not edit everything in it, and a carve rewrites the members
    // themselves. Checked before anything is written, so a bundle with one
    // untouchable member fails loudly rather than being carved most of the way.
    for subject in &subjects {
        user.require_can_edit(subject.created_by)?;
    }

    let mut result = BundleCarveResult {
        members_carved: 0,
        models_created: 0,
        variants_removed: 0,
    };
    let mut created: Vec<Uuid> = Vec::new();
    for subject in &subjects {
        let outcome = carve_one(&mut tx, subject, &spec, &user).await?;
        if outcome.carved == 0 {
            continue;
        }
        result.members_carved += 1;
        result.variants_removed += outcome.variants_removed;
        created.extend(outcome.created);
    }
    if result.members_carved == 0 {
        return Err(ApiError::BadRequest(
            "no file on any member matches this layout — there is nothing to carve".into(),
        ));
    }
    result.models_created = created.len();

    sqlx::query!(
        "UPDATE bundles SET updated_at = now() WHERE id = $1",
        bundle_id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    for model_id in &created {
        enqueue_previews(&state, *model_id).await?;
    }

    tracing::info!(
        bundle = %bundle_id,
        members = subjects.len(),
        carved = result.members_carved,
        created = result.models_created,
        variants_removed = result.variants_removed,
        "carved a bundle's members",
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
