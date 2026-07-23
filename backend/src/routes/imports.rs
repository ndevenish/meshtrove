//! Imports: the staging area a dropped archive lands in.
//!
//! An import is neither a model nor a bundle — it never shows up in browse or
//! search, only on the "Importing" list. Files uploaded to it are owned by it
//! (`files.import_id`), so a zip can upload and unpack with no decision made
//! about what it *is*. `POST /api/imports/{id}/commit` then moves every staged
//! file onto exactly one destination — a new model, a new bundle, or an existing
//! bundle — and drops the import row.

use std::collections::{HashMap, HashSet};

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
use crate::routes::bundles;
use crate::routes::custom_fields::{self, CustomFieldValueInput};
use crate::routes::models;
use crate::services::gc;
use crate::services::layout::{self, CarveTarget, LayoutSpec, Plan, PlanVariant};
use crate::state::AppState;
use crate::util::{slug_token, slugify};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/imports", get(list).post(create))
        .route("/api/imports/{id}", get(detail).put(update).delete(remove))
        .route("/api/imports/{id}/split", post(split))
        .route("/api/imports/{id}/discard", post(discard))
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
    /// The import is still filling up — an unpack job for one of its archives is
    /// queued or running, or a dropbox pickup is still copying entries onto it.
    /// Either way the contents aren't final yet, so committing is refused.
    pub unpacking: bool,
    /// Archives of this import still waiting to be opened, or being opened now.
    /// Counts down as the unpack works through them — and *up* whenever one of
    /// them turns out to hold more archives, which is honest: nobody can know
    /// the shape of a pack of packs until it has been opened.
    pub archives_left: i64,
    /// Files staged by the jobs running on this import right now, out of what
    /// those jobs have left to do. Not the import's total: an unpack knows its
    /// own archive's file count once it has extracted it (see importer.rs), and
    /// what the *next* archive holds is nobody's business yet. Both 0 when
    /// nothing is running or nothing has reported.
    pub staging_done: i64,
    pub staging_total: i64,
    /// The dropped archive is a MeshTrove export: it is restored (recreating the
    /// models/bundles it holds), not carved. The Import page shows a restore
    /// panel rather than the layout UI.
    pub is_export: bool,
    /// A "keep unmatched files" carve has already placed some of this import:
    /// what's staged now is the remainder, awaiting another pass.
    pub partial: bool,
}

/// An import's files are listed via `GET /api/imports/{id}/files` (files.rs),
/// like every other file owner.
async fn list(
    State(state): State<AppState>,
    user: User,
) -> Result<Json<Vec<ImportSummary>>, ApiError> {
    user.require_editor()?;
    let rows = sqlx::query!(
        r#"SELECT i.id, i.name, i.created_by, i.created_at, i.is_export, i.partial,
                  (SELECT count(*) FROM files f WHERE f.import_id = i.id) as "file_count!",
                  w.jobs_left > 0 as "unpacking!",
                  w.archives_left as "archives_left!",
                  w.staging_done as "staging_done!",
                  w.staging_total as "staging_total!"
           FROM imports i
           LEFT JOIN LATERAL (SELECT * FROM import_work(i.id)) w ON true
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
                archives_left: r.archives_left,
                staging_done: r.staging_done,
                staging_total: r.staging_total,
                is_export: r.is_export,
                partial: r.partial,
            })
            .collect(),
    ))
}

pub async fn fetch_import(state: &AppState, id: Uuid) -> Result<ImportSummary, ApiError> {
    let r = sqlx::query!(
        r#"SELECT i.id, i.name, i.created_by, i.created_at, i.is_export, i.partial,
                  (SELECT count(*) FROM files f WHERE f.import_id = i.id) as "file_count!",
                  w.jobs_left > 0 as "unpacking!",
                  w.archives_left as "archives_left!",
                  w.staging_done as "staging_done!",
                  w.staging_total as "staging_total!"
           FROM imports i
           LEFT JOIN LATERAL (SELECT * FROM import_work(i.id)) w ON true
           WHERE i.id = $1"#,
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
        archives_left: r.archives_left,
        staging_done: r.staging_done,
        staging_total: r.staging_total,
        is_export: r.is_export,
        partial: r.partial,
    })
}

async fn detail(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<ImportSummary>, ApiError> {
    // An import is editor-and-above working state — never public. Gate viewing the
    // same way `list` does (editor+), so a signed-out visitor (a guest viewer)
    // can't read it. The write paths (plan/commit/update/remove) still gate on
    // ownership via `require_can_edit`.
    user.require_editor()?;
    Ok(Json(fetch_import(&state, id).await?))
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<ImportInput>,
) -> Result<Json<ImportSummary>, ApiError> {
    user.require_editor()?;
    Ok(Json(create_import(&state, &user, &input.name).await?))
}

/// Open an empty import. Shared with the dropbox pickup (routes/dropbox.rs),
/// which creates the import up front and fills it from a background job — the
/// caller has already decided who is allowed to do that.
pub async fn create_import(
    state: &AppState,
    user: &User,
    name: &str,
) -> Result<ImportSummary, ApiError> {
    let name = name.trim();
    let name = if name.is_empty() { "Import" } else { name };
    let id: Uuid = sqlx::query_scalar!(
        "INSERT INTO imports (name, created_by) VALUES ($1, $2) RETURNING id",
        name,
        user.id,
    )
    .fetch_one(&state.db)
    .await?;
    fetch_import(state, id).await
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

/// Whether `folder` sits inside `ancestor` — the same folder, or nested under
/// it. Segment-wise so `Loot/A` is not read as being under `Loot/AB`, which a
/// plain `starts_with` would get wrong.
fn is_under(folder: &str, ancestor: &str) -> bool {
    folder == ancestor || folder.starts_with(&format!("{ancestor}/"))
}

/// The longest run of leading path segments every folder shares — the deepest
/// directory they all sit under. Compared segment by segment for the same reason
/// as `is_under`: a shared string prefix is not a shared folder.
fn common_prefix(folders: &[String]) -> Vec<&str> {
    let mut prefix: Vec<&str> = folders
        .first()
        .map(|f| f.split('/').collect())
        .unwrap_or_default();
    for folder in &folders[1..] {
        let shared = folder
            .split('/')
            .zip(&prefix)
            .take_while(|(seg, kept)| seg == *kept)
            .count();
        prefix.truncate(shared);
    }
    prefix
}

#[derive(Deserialize, ToSchema)]
pub struct SplitInput {
    /// The staged folders to lift out, as they appear in the tree — the files
    /// directly in each and everything under it move together. One is the common
    /// case (carving a product off a back-catalogue drop); several at once is for
    /// gathering scattered folders into one import in a single move.
    pub folders: Vec<String>,
    /// What to call the new import. Defaults to the folder's own name for a lone
    /// folder, and to the name of the folder the selection sits under otherwise.
    #[serde(default)]
    pub name: Option<String>,
}

/// Lift one or more staged folders out into an import of their own.
///
/// One drop is routinely several things — a dropbox pickup of a creator's whole
/// back catalogue is a folder per product — and an import commits to exactly one
/// destination. Splitting is how that drop becomes the several imports it always
/// was, without a re-upload: the file rows change owner, the blobs never move.
///
/// A single folder becomes the new import's top directory and its ancestors are
/// dropped: `Loot/KingIn_Yellow/images/a.jpg` splits to `KingIn_Yellow/images/a.jpg`.
/// With several folders selected at once, the deepest folder they *all* sit under
/// is what gets dropped, so each keeps its own name at the top and they stay
/// distinct: `Loot/A/x` and `Loot/B/y` split to `A/x` and `B/y`. Either way the
/// path that identified the *drop* has no meaning in the smaller import, while the
/// folder names that carry what each thing *is* — what a layout's model-name
/// capture reads — are preserved.
///
/// Nested selections collapse to their outermost folder: picking `Loot` and
/// `Loot/A` moves `Loot`, since `A` is already inside it.
///
/// Refused while the source is still unpacking: half a folder is staged at that
/// point, and the rest would land in the import this one just left.
async fn split(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<SplitInput>,
) -> Result<Json<ImportSummary>, ApiError> {
    let source = fetch_import(&state, id).await?;
    user.require_can_edit(source.created_by)?;
    if source.unpacking {
        return Err(ApiError::BadRequest(
            "this import is still unpacking — wait for it to finish before splitting it".into(),
        ));
    }
    if source.is_export {
        return Err(ApiError::BadRequest(
            "an export is restored whole, not split".into(),
        ));
    }
    // Trimmed the way a folder header shows it, and the way `path` is stored:
    // no leading or trailing slashes. Empty means the root, which is the whole
    // import — not a folder to carve out.
    let mut folders: Vec<String> = input
        .folders
        .iter()
        .map(|f| f.trim().trim_matches('/').to_string())
        .collect();
    if folders.is_empty() || folders.iter().any(String::is_empty) {
        return Err(ApiError::BadRequest(
            "the whole import is not a folder to split out".into(),
        ));
    }
    // Collapse the selection to its outermost folders: a folder listed alongside
    // one of its own ancestors is already carried by that ancestor, and leaving
    // it in would have its files matched (and their paths rewritten) twice.
    folders.sort();
    folders.dedup();
    folders = folders
        .iter()
        .filter(|f| {
            !folders
                .iter()
                .any(|a| a.as_str() != f.as_str() && is_under(f, a))
        })
        .cloned()
        .collect();

    // The prefix every moved path loses. For a lone folder that is its parent, so
    // the folder itself stays as the top directory. For several it is the deepest
    // directory they all share, so each keeps its own name and they stay apart.
    let prefix_segs: Vec<&str> = if folders.len() == 1 {
        let segs: Vec<&str> = folders[0].split('/').collect();
        segs[..segs.len() - 1].to_vec()
    } else {
        common_prefix(&folders)
    };
    let prefix = if prefix_segs.is_empty() {
        String::new()
    } else {
        format!("{}/", prefix_segs.join("/"))
    };
    // The default name says what the new import holds: the lone folder's own name,
    // or — for a gathered selection — the folder they were all sitting under.
    let default_name = if folders.len() == 1 {
        folders[0]
            .rsplit('/')
            .next()
            .unwrap_or(&folders[0])
            .to_string()
    } else {
        prefix_segs
            .last()
            .map(|s| s.to_string())
            .unwrap_or_else(|| source.name.clone())
    };
    let name = match input.name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => default_name,
    };

    let mut tx = state.db.begin().await?;
    // The new import belongs to whoever owned the drop, not to whoever pressed
    // the button: it is the same staged work, carried on in a second window.
    let new_id: Uuid = sqlx::query_scalar!(
        "INSERT INTO imports (name, created_by, partial) VALUES ($1, $2, $3) RETURNING id",
        name,
        source.created_by,
        // The split half is as partly-placed as the whole was.
        source.partial,
    )
    .fetch_one(&mut *tx)
    .await?;
    // `left(path, n) = folder || '/'` rather than LIKE: a folder name is free to
    // contain `%` or `_`, and those are not wildcards here. Every selected folder
    // is tested against each row through `unnest`, so one statement moves them all;
    // the shared prefix they lose is a fixed width, so `substring` is the same cut
    // for every match.
    let moved = sqlx::query_scalar!(
        r#"WITH moved AS (
             UPDATE files
                SET import_id = $2,
                    path = substring(path from $4)
              WHERE import_id = $1
                AND EXISTS (
                  SELECT 1 FROM unnest($3::text[]) AS f(folder)
                   WHERE path = f.folder OR left(path, length(f.folder) + 1) = f.folder || '/'
                )
              RETURNING 1
           )
           SELECT count(*) as "count!" FROM moved"#,
        id,
        new_id,
        &folders,
        // 1-based, and in characters: `substring` counts characters, and a
        // folder name is free to hold anything a filesystem allows.
        (prefix.chars().count() + 1) as i32,
    )
    .fetch_one(&mut *tx)
    .await?;
    if moved == 0 {
        tx.rollback().await?;
        return Err(ApiError::BadRequest(format!(
            "nothing is staged under {}",
            folders.join(", ")
        )));
    }
    // What was typed about the drop is true of both halves of it: the licence,
    // the invoice PDF, the creator's terms. The source keeps its values (it is
    // still an import to be committed) and the new one starts with a copy —
    // a file-kind value's blob is shared, so the copy is a row, not bytes.
    let staged = custom_fields::values_of(&mut tx, custom_fields::ValueOwner::Import(id)).await?;
    custom_fields::copy_values_onto(
        &mut tx,
        &staged,
        custom_fields::ValueOwner::Import(new_id),
        |_| true,
        &user,
    )
    .await?;
    tx.commit().await?;

    tracing::info!(
        from = %id, to = %new_id, folders = %folders.join(", "), files = moved,
        "split folders out of an import"
    );
    Ok(Json(fetch_import(&state, new_id).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct DiscardInput {
    /// The staged folder to drop, as it appears in the tree. Empty is the
    /// import's root — the files sitting in no folder at all.
    pub folder: String,
    /// Take the folders under it too. False drops only what sits directly at
    /// this path and leaves the subtree staged, which is the choice the tree
    /// offers for a folder that has folders beneath it.
    #[serde(default)]
    pub tree: bool,
}

#[derive(Serialize, ToSchema)]
pub struct DiscardResult {
    pub deleted: i64,
}

/// Drop a staged folder without importing it — the chaff an archive arrives
/// wrapped in, deleted before the carve rather than after.
///
/// One request, not one per file. The page used to delete a folder by sending a
/// DELETE for every file in it, eight at a time; on the folders this is actually
/// for — thousands of files — that is thousands of round trips, and it buried
/// the server under work that is a single statement here. It is also the only
/// form that still works once the page stopped holding every staged file: the
/// tree knows a folder's *count* long before it knows its file ids, and it must
/// be able to discard one it has never loaded.
async fn discard(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<DiscardInput>,
) -> Result<Json<DiscardResult>, ApiError> {
    let import = fetch_import(&state, id).await?;
    user.require_can_edit(import.created_by)?;
    if import.unpacking {
        return Err(ApiError::BadRequest(
            "this import is still unpacking — wait for it to finish before discarding from it"
                .into(),
        ));
    }
    let folder = input.folder.trim().trim_matches('/').to_string();
    // The root is a folder here, and its files can be dropped like any other's —
    // but "the root and everything under it" is the whole import, which is
    // `DELETE /api/imports/{id}` and should say so rather than arriving disguised
    // as tidying up a folder.
    if folder.is_empty() && input.tree {
        return Err(ApiError::BadRequest(
            "that would discard the entire import — delete the import instead".into(),
        ));
    }
    // Same `left(path, n)` test as `split`, and for the same reason: a folder
    // name may hold `%` or `_`, which LIKE would read as wildcards.
    let deleted = sqlx::query_scalar!(
        r#"WITH gone AS (
             DELETE FROM files
              WHERE import_id = $1
                AND (path = $2 OR ($3 AND left(path, length($2) + 1) = $2 || '/'))
              RETURNING 1
           )
           SELECT count(*) as "count!" FROM gone"#,
        id,
        folder,
        input.tree,
    )
    .fetch_one(&state.db)
    .await?;

    tracing::info!(import = %id, %folder, tree = input.tree, files = deleted, "discarded a staged folder");
    Ok(Json(DiscardResult { deleted }))
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
    /// The existing bundle a "bundle" carve is being merged into, if any. With
    /// it the plan reports, per member model, which existing member it would
    /// merge onto (`merge_target`) and the members available to retarget to.
    #[serde(default)]
    pub bundle_id: Option<Uuid>,
    /// Answer with the tallies and the planned shape, but not the per-file
    /// annotations — for a caller that only wants to know how much a layout
    /// would match.
    ///
    /// The annotations are one entry per staged file and are essentially the
    /// whole response: on a 42k-file import a plan is some 10 MB, and the
    /// layout picker dry-runs *every saved layout* on page load to rank them by
    /// coverage, reading a single integer from each. That is what this is for.
    #[serde(default)]
    pub counts_only: bool,
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
    let mut plan = layout::analyze(&request.spec, request.target, &files, &vocab)?;

    // Merging into an existing bundle: annotate each planned model with the
    // member the carve would land it on (by name/alias + tag coverage), and list
    // every member so the UI can offer a retarget dropdown. Same `match_member`
    // the commit's carve uses, so the preview matches the result.
    if request.target == CarveTarget::Bundle
        && let Some(bundle_id) = request.bundle_id
    {
        let members = bundle_members(&state.db, bundle_id).await?;
        for planned in &mut plan.models {
            planned.merge_target = match_member(&members, planned).map(|m| m.id);
        }
        plan.members = members
            .into_iter()
            .map(|m| layout::MemberCandidate {
                id: m.id,
                name: m.name,
                tags: m.tags,
            })
            .collect();
    }

    // Dropped after the analysis, not skipped during it: these are how `analyze`
    // reports what it did, and the tallies are counted off the same pass. Only
    // the serialising of them is worth avoiding. Everything that grows with the
    // size of the drop goes — the annotations per file, the planned models per
    // captured name — leaving the tallies and the per-rule breakdown.
    if request.counts_only {
        plan.annotations = Vec::new();
        plan.models = Vec::new();
        plan.model_names = Vec::new();
        plan.members = Vec::new();
    }

    Ok(Json(plan))
}

/// One member model of a bundle, with everything the merge match needs: its
/// name, the alternate names it answers to (aliases), and its model tags.
struct MemberRow {
    id: Uuid,
    name: String,
    tags: Vec<String>,
    aliases: Vec<String>,
}

/// Every member model of a bundle. Reused by the plan endpoint (to preview merge
/// targets) and the commit's carve (to actually reuse members).
async fn bundle_members(
    db: impl sqlx::PgExecutor<'_>,
    bundle_id: Uuid,
) -> Result<Vec<MemberRow>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT m.id, m.name::text as "name!",
                  coalesce((SELECT array_agg(t.name::text) FROM model_tags mt
                            JOIN tags t ON t.id = mt.tag_id WHERE mt.model_id = m.id), '{}')
                      as "tags!: Vec<String>",
                  coalesce((SELECT array_agg(a.alias::text) FROM model_aliases a
                            WHERE a.model_id = m.id), '{}')
                      as "aliases!: Vec<String>"
           FROM models m JOIN bundle_models bm ON bm.model_id = m.id
           WHERE bm.bundle_id = $1
           ORDER BY m.name"#,
        bundle_id,
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| MemberRow {
            id: r.id,
            name: r.name,
            tags: r.tags,
            aliases: r.aliases,
        })
        .collect())
}

/// The member a planned model merges onto by default: its name (or any alias)
/// matches, and the member already carries every model tag the plan captured. A
/// past import may have taught a member an alternate name, so a re-drop that
/// spells it differently still finds it.
fn match_member<'a>(
    members: &'a [MemberRow],
    planned: &layout::PlanModel,
) -> Option<&'a MemberRow> {
    let name = planned.name.to_lowercase();
    let wanted: Vec<String> = planned.tags.iter().map(|t| t.to_lowercase()).collect();
    members.iter().find(|m| {
        let name_hit =
            m.name.to_lowercase() == name || m.aliases.iter().any(|a| a.to_lowercase() == name);
        if !name_hit {
            return false;
        }
        let mtags: Vec<String> = m.tags.iter().map(|t| t.to_lowercase()).collect();
        wanted.iter().all(|t| mtags.contains(t))
    })
}

async fn plan_files(
    db: impl sqlx::PgExecutor<'_>,
    import_id: Uuid,
) -> Result<Vec<layout::PlanFile>, ApiError> {
    // The archive itself is staged alongside its contents, but it is never
    // carved — it becomes a source_archives row at commit. Matching the carve
    // against its filename can only fail, which would drag the match count down
    // by one against a file that was never a candidate.
    let rows = sqlx::query!(
        "SELECT id, path, filename FROM files
         WHERE import_id = $1 AND kind <> 'archive'::file_kind
         ORDER BY path, filename",
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
    /// Becomes the first description revision of whatever the import targets:
    /// the model under a model target, the *bundle* under a bundle target. It
    /// is not fanned out to member models — see `apply_meta_bulk`.
    pub description_md: Option<String>,
    /// Admin-defined extra fields typed on the import page. Each value goes
    /// wherever its own definition says it belongs — see `apply_custom_fields`.
    #[serde(default)]
    pub custom_fields: Vec<CustomFieldValueInput>,
}

/// Route the import's custom field values by each field's own settings.
///
/// Two sources, one routing rule. The scalars arrive in the commit body, typed
/// on the page moments ago; a file field's bytes could not wait for a
/// destination and were staged on the import when they were dropped, so they
/// are read back off it here. Both then go the same way.
///
/// A model target is simple: everything that applies to models lands on it. A
/// bundle target has two halves — a field that applies to bundles is the box
/// set's own, written on the bundle and then reaching its members exactly as far
/// as its "persists to model" / "overwrites" flags say; a models-only field has
/// no bundle to live on, so it is written on every model the carve produced, the
/// same way the typed tags are.
async fn apply_custom_fields(
    tx: &mut sqlx::PgConnection,
    import_id: Uuid,
    meta: &ImportMeta,
    bundle_id: Option<Uuid>,
    model_ids: &[Uuid],
    user: &User,
) -> Result<(), ApiError> {
    let staged = custom_fields::values_of(tx, custom_fields::ValueOwner::Import(import_id)).await?;
    if meta.custom_fields.is_empty() && staged.is_empty() {
        return Ok(());
    }
    match bundle_id {
        Some(bundle_id) => {
            let on_bundle =
                custom_fields::resolve_values(tx, &meta.custom_fields, |f| f.applies_to_bundles)
                    .await?;
            custom_fields::write_bundle_values(tx, bundle_id, &on_bundle, user).await?;
            custom_fields::copy_values_onto(
                tx,
                &staged,
                custom_fields::ValueOwner::Bundle(bundle_id),
                |v| v.applies_to_bundles,
                user,
            )
            .await?;
            // After both, so a staged file reaches the members the same way a
            // typed value does.
            custom_fields::persist_bundle_fields(tx, bundle_id, user).await?;
            // Only the fields the bundle couldn't hold: a field that lives at
            // both ends already reached the members through persistence (or was
            // deliberately not configured to).
            let on_members = custom_fields::resolve_values(tx, &meta.custom_fields, |f| {
                f.applies_to_models && !f.applies_to_bundles
            })
            .await?;
            custom_fields::write_model_values_bulk(tx, model_ids, &on_members, user).await?;
            for &model_id in model_ids {
                custom_fields::copy_values_onto(
                    tx,
                    &staged,
                    custom_fields::ValueOwner::Model(model_id),
                    |v| v.applies_to_models && !v.applies_to_bundles,
                    user,
                )
                .await?;
            }
        }
        None => {
            let on_models =
                custom_fields::resolve_values(tx, &meta.custom_fields, |f| f.applies_to_models)
                    .await?;
            custom_fields::write_model_values_bulk(tx, model_ids, &on_models, user).await?;
            for &model_id in model_ids {
                custom_fields::copy_values_onto(
                    tx,
                    &staged,
                    custom_fields::ValueOwner::Model(model_id),
                    |v| v.applies_to_models,
                    user,
                )
                .await?;
            }
        }
    }
    Ok(())
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
    /// metadata lands on the bundle *and* on every member the carve creates —
    /// except the description, which is the box set's own and stays on it.
    NewBundle {
        name: Option<String>,
        /// The name field was left at the archive's own name, not typed by the
        /// user — so a later metadata import may replace it with the real title.
        #[serde(default)]
        name_autogenerated: bool,
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
        /// Per planned-model retarget choices from the "Will become" dropdowns,
        /// index-aligned to `plan.models`: `Some(member_id)` merges onto that
        /// member, `null` creates a new one. Empty (the default) falls back to
        /// the automatic name/alias match.
        #[serde(default)]
        merge_targets: Vec<Option<Uuid>>,
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

    // Dry-run the layout carve first: a bad pattern or an unmapped value must
    // fail the commit before anything is created. Same `analyze` as the plan
    // endpoint, so the preview the user confirmed is exactly what happens.
    let (carve_target, layout_spec) = match &input {
        CommitInput::NewModel { layout, .. } => (CarveTarget::Model, layout),
        CommitInput::NewBundle { layout, .. } | CommitInput::Bundle { layout, .. } => {
            (CarveTarget::Bundle, layout)
        }
    };
    // Commit only what the rules match: unmatched files stay staged on the
    // import, which survives the commit (flagged partial) for another pass.
    let keep_unmatched = layout_spec.as_ref().is_some_and(|spec| spec.keep_unmatched);

    // What the import was unpacked *from*. Read while the files still carry
    // `import_id`; the rows themselves are dropped further down, once the commit
    // knows which model or bundle to hang the provenance off. On a partial
    // commit the archive isn't redundant yet — it stays staged with the
    // remainder, and its provenance follows whichever commit finally empties
    // the import.
    let archives = if keep_unmatched {
        Vec::new()
    } else {
        gc::redundant_archives(&mut tx, id).await?
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

    // The files this commit claims off the import: everything (None), or — when
    // keeping unmatched files — exactly the ones the plan matched. Every matched
    // file leaves the import: carved ones onto their variants, matched-but-
    // uncarvable ones (no model name under a bundle target) into unsorted.
    let claimed: Option<Vec<Uuid>> = if keep_unmatched {
        let plan = carve.as_ref().expect("keep_unmatched implies a layout");
        let ids: Vec<Uuid> = plan
            .annotations
            .iter()
            .filter(|a| a.matched)
            .map(|a| a.id)
            .collect();
        if ids.is_empty() {
            return Err(ApiError::BadRequest(
                "no staged file matches the layout — nothing would be imported".into(),
            ));
        }
        Some(ids)
    } else {
        None
    };

    // The folders a carve has already read. Collected *before* anything moves,
    // because the carve matches on `path` — flattening first would pull the tree
    // out from under the pattern that is reading it — and because a file on its
    // way to a variant no longer has an `import_id` to find it by afterwards.
    // Only the claimed files flatten: what stays staged keeps its tree, or the
    // next pass would have nothing left to match on.
    let flatten_ids: Vec<Uuid> = if layout_spec.as_ref().is_some_and(|spec| spec.flatten) {
        match &claimed {
            Some(ids) => ids.clone(),
            None => {
                sqlx::query_scalar!("SELECT id FROM files WHERE import_id = $1", id)
                    .fetch_all(&mut *tx)
                    .await?
            }
        }
    } else {
        Vec::new()
    };

    // Resolve every tag name the carve and the typed metadata need in two bulk
    // upserts (one per vocabulary), so the per-model work below is pure inserts
    // against a name→id map rather than an INSERT-or-SELECT round trip per tag —
    // the same handful of variant tags recur across every member of a big bundle.
    let meta = match &input {
        CommitInput::NewModel { meta, .. }
        | CommitInput::NewBundle { meta, .. }
        | CommitInput::Bundle { meta, .. } => meta,
    };
    let mut vtag_names: Vec<String> = Vec::new();
    let mut mtag_names: Vec<String> = meta.tags.clone();
    if let Some(plan) = &carve {
        for planned in &plan.models {
            mtag_names.extend(planned.tags.iter().cloned());
            for variant in &planned.variants {
                vtag_names.extend(variant.tags.iter().cloned());
            }
        }
        mtag_names.extend(plan.model_tag_order.iter().cloned());
    }
    let tags = TagMaps {
        variant: upsert_variant_tags_bulk(&mut tx, &vtag_names).await?,
        model: upsert_tags_bulk(&mut tx, &mtag_names).await?,
    };

    // Models whose browse thumbnail should render once the commit lands.
    let mut render_models: Vec<Uuid> = Vec::new();
    // Models a bundle carve created, for the metadata that belongs on members
    // rather than on the bundle.
    let mut members: Vec<Uuid> = Vec::new();

    let result = match &input {
        CommitInput::NewModel { name, meta, .. } => {
            let name = named(name);
            let slug = models::unique_slug(&state, &name, None, None).await?;
            // A one-model carve produces a single planned model; take the creator
            // id and version its layout caught, if any.
            let planned_model = carve.as_ref().and_then(|p| p.models.first());
            let creator_ref = planned_model.and_then(|m| m.creator_ref.clone());
            let model_version = planned_model.and_then(|m| m.model_version.clone());
            let model_id: Uuid = sqlx::query_scalar!(
                "INSERT INTO models (name, slug, creator_id, creator_ref, model_version, created_by)
                 VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
                name,
                slug,
                meta.creator_id,
                creator_ref,
                model_version,
                user.id,
            )
            .fetch_one(&mut *tx)
            .await?;
            apply_meta_bulk(&mut tx, &[model_id], meta, user.id, &tags.model, true).await?;
            apply_custom_fields(&mut tx, id, meta, None, &[model_id], &user).await?;
            if let Some(plan) = &carve
                && let Some(planned) = plan.models.first()
            {
                add_model_tags(&mut tx, model_id, &planned.tags, &tags.model).await?;
                carve_variants(
                    &mut tx,
                    model_id,
                    &planned.variants,
                    user.id,
                    Untagged::UnsortedBucket,
                    &tags.variant,
                    true,
                )
                .await?;
            }
            // Whatever the carve didn't claim (or all of it, with no layout)
            // lands in the model's unsorted bucket — unless unmatched files are
            // staying staged, in which case only the matched ones move.
            claim_files(&mut tx, id, Some(model_id), None, claimed.as_deref()).await?;
            // Any images among those files are pictures of the model — pull them
            // out of the file list and into its gallery.
            adopt_model_images(&mut tx, model_id, user.id).await?;
            render_models.push(model_id);
            CommitResult {
                kind: "model".into(),
                id: model_id,
                slug,
            }
        }
        CommitInput::NewBundle {
            name,
            meta,
            name_autogenerated,
            ..
        } => {
            let name = named(name);
            let slug = bundles::unique_slug(&state, &name, None, None).await?;
            let bundle_id: Uuid = sqlx::query_scalar!(
                "INSERT INTO bundles (name, slug, creator_id, source_url, name_autogenerated, created_by)
                 VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
                name,
                slug,
                meta.creator_id,
                meta.source_url,
                *name_autogenerated,
                user.id,
            )
            .fetch_one(&mut *tx)
            .await?;
            if let Some(plan) = &carve {
                // A brand-new bundle has no members yet, so no merge decision:
                // every planned model is created fresh.
                let created = carve_into_bundle(
                    &mut tx,
                    bundle_id,
                    meta.creator_id,
                    plan,
                    user.id,
                    &[],
                    &tags,
                )
                .await?;
                // The box set was bought once: what was typed on the import page
                // is true of every model the carve just pulled out of it — the
                // description excepted, which describes the box, not each figure.
                apply_meta_bulk(&mut tx, &created, meta, user.id, &tags.model, false).await?;
                members.extend(created.iter().copied());
                render_models.extend(created);
            }
            apply_custom_fields(&mut tx, id, meta, Some(bundle_id), &members, &user).await?;
            apply_bundle_description(&mut tx, bundle_id, meta, user.id).await?;
            claim_files(&mut tx, id, None, Some(bundle_id), claimed.as_deref()).await?;
            CommitResult {
                kind: "bundle".into(),
                id: bundle_id,
                slug,
            }
        }
        CommitInput::Bundle {
            bundle_id,
            meta,
            merge_targets,
            ..
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
                let created = carve_into_bundle(
                    &mut tx,
                    *bundle_id,
                    creator,
                    plan,
                    user.id,
                    merge_targets,
                    &tags,
                )
                .await?;
                // Only the models this drop *created*: a member that was already
                // in the bundle has its own metadata, and a later 75mm pack has no
                // business rewriting it.
                apply_meta_bulk(&mut tx, &created, meta, user.id, &tags.model, false).await?;
                members.extend(created.iter().copied());
                render_models.extend(created);
            }
            apply_custom_fields(&mut tx, id, meta, Some(*bundle_id), &members, &user).await?;
            apply_bundle_description(&mut tx, *bundle_id, meta, user.id).await?;
            claim_files(&mut tx, id, None, Some(*bundle_id), claimed.as_deref()).await?;
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

    // A `folder` capture rewrites the path outright — the general case of the
    // flatten above, which rewrites it to nothing. Applied second so it wins for
    // the files that resolved one: the drop's useless top folder goes, the
    // meaningful one under it stays. One statement per distinct folder, not per
    // file. Same claimed-files rule as flatten, for the same reason: what stays
    // staged must keep the tree the next pass will match on.
    if let Some(plan) = &carve {
        for (folder, ids) in plan.folder_moves(claimed.as_deref()) {
            sqlx::query!(
                "UPDATE files SET path = $2 WHERE id = ANY($1::uuid[])",
                &ids[..],
                folder,
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    // Both rewrites above are blind, and that is what makes this necessary: a
    // flatten sends every claimed file to `''` and a `folder` capture sends a
    // whole subtree to one name, so two files that were only ever told apart by
    // the folder they sat in now agree on `(owner, path, filename)`. Run last,
    // once every move and rewrite has settled, so it also catches a claimed file
    // landing on a name the destination already had.
    let owner_models: Vec<Uuid> = match result.kind.as_str() {
        "model" => render_models
            .iter()
            .copied()
            .chain(std::iter::once(result.id))
            .collect(),
        _ => render_models.clone(),
    };
    let owner_bundle = match result.kind.as_str() {
        "model" => None,
        _ => Some(result.id),
    };
    disambiguate_filenames(&mut tx, &owner_models, owner_bundle).await?;

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

    if keep_unmatched {
        // The unmatched files are still staged, so the import lives on — marked
        // partial, so the Importing list can say some of it is already placed.
        // The next commit can target anything, including a different bundle.
        sqlx::query!(
            "UPDATE imports SET partial = true, updated_at = now() WHERE id = $1",
            id,
        )
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query!("DELETE FROM imports WHERE id = $1", id)
            .execute(&mut *tx)
            .await?;
    }
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
        // ...but only when the model has no picture yet. An archive that shipped
        // photos (now the model's own images) already has something to show, and a
        // rendered STL stand-in behind it is wasted work — so skip the render
        // entirely if any image already hangs off the model or its variants.
        let has_image = sqlx::query_scalar!(
            r#"SELECT EXISTS (
                   SELECT 1 FROM images i
                   LEFT JOIN model_variants v ON v.id = i.variant_id
                   WHERE i.model_id = $1 OR v.model_id = $1
               ) AS "exists!""#,
            model_id,
        )
        .fetch_one(&state.db)
        .await?;
        if has_image {
            continue;
        }

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

/// Move the files a commit claims off the import and onto its destination —
/// exactly one of `model_id`/`bundle_id`. `only = None` claims everything still
/// staged; `Some(ids)` (a "keep unmatched files" carve) claims just those,
/// leaving the rest owned by the import. Files the carve already placed on
/// variants have no `import_id` any more, so they're untouched either way.
async fn claim_files(
    tx: &mut sqlx::PgConnection,
    import_id: Uuid,
    model_id: Option<Uuid>,
    bundle_id: Option<Uuid>,
    only: Option<&[Uuid]>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE files SET model_id = $2, bundle_id = $3, import_id = NULL
         WHERE import_id = $1 AND ($4::uuid[] IS NULL OR id = ANY($4))",
        import_id,
        model_id,
        bundle_id,
        only,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Give every committed file a name unique within its owner, suffixing ` (2)`,
/// ` (3)`, … before the extension — the same convention export uses for
/// colliding readable paths (see [`crate::services::transfer`]).
///
/// Renaming, not de-duplicating: a clash here is between two files the user
/// asked for, which happened to share a name in different folders. Dropping one
/// would lose geometry. What the rename buys is that nothing downstream has to
/// guess — two files with one name under one owner list twice identically,
/// collide as one entry on export, and make "delete that file" a coin toss.
///
/// Staged files are exempt by construction: this runs only over the commit's
/// destination owners, never over an import. An import may legitimately hold a
/// clash while its archives unpack, and settling it there would fight the next
/// carve, which matches on the paths this would rewrite.
///
/// Looped because a generated name can itself be taken — an owner holding
/// `Bolt.stl` twice *and* a real `Bolt (2).stl` needs a second pass. Each pass
/// strictly lengthens the names it touches, so this terminates; the bound is
/// there to make that a fact about the code rather than about the argument.
async fn disambiguate_filenames(
    tx: &mut sqlx::PgConnection,
    models: &[Uuid],
    bundle: Option<Uuid>,
) -> Result<(), ApiError> {
    for _ in 0..16 {
        let renamed = sqlx::query!(
            r#"WITH ranked AS (
                   SELECT f.id, f.filename,
                          row_number() OVER (
                              PARTITION BY f.model_id, f.variant_id, f.bundle_id,
                                           f.path, f.filename
                              ORDER BY f.created_at, f.id
                          ) AS rn
                   FROM files f
                   LEFT JOIN model_variants v ON v.id = f.variant_id
                   WHERE f.model_id = ANY($1)
                      OR v.model_id = ANY($1)
                      OR ($2::uuid IS NOT NULL AND f.bundle_id = $2)
               )
               UPDATE files f
               SET filename = CASE
                       WHEN r.filename ~ '\.[^.]+$'
                       THEN regexp_replace(r.filename, '^(.*)\.([^.]+)$',
                                           '\1 (' || r.rn || ').\2')
                       ELSE r.filename || ' (' || r.rn || ')'
                   END
               FROM ranked r
               WHERE f.id = r.id AND r.rn > 1"#,
            models,
            bundle,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if renamed == 0 {
            return Ok(());
        }
    }
    Err(ApiError::Internal(anyhow::anyhow!(
        "could not settle colliding filenames after 16 passes"
    )))
}

// ---------------------------------------------------------------------------
// bulk tag get-or-create: resolve every tag name a commit needs in one round
// trip each, keyed by lowercased name. A big bundle carve touches the same
// handful of variant tags ("32mm", "supported") across dozens of models; doing
// them one INSERT-or-SELECT at a time was the bulk of the commit's DB chatter.
// ---------------------------------------------------------------------------

/// The tag vocabularies a commit pre-resolves up front (name → id, keyed
/// lowercased), so the per-model carve is map lookups rather than a round trip
/// per tag.
struct TagMaps {
    variant: HashMap<String, Uuid>,
    model: HashMap<String, Uuid>,
}

/// Get-or-create every `variant_tags` name, returning name (trimmed, lowercased)
/// → id. `name` is citext, so the map is case-insensitive; trim in Rust so
/// " 32mm" and "32mm" collapse the way the single-name upsert does.
async fn upsert_variant_tags_bulk(
    tx: &mut sqlx::PgConnection,
    names: &[String],
) -> Result<HashMap<String, Uuid>, ApiError> {
    let cleaned: Vec<String> = names
        .iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    if cleaned.is_empty() {
        return Ok(HashMap::new());
    }
    // Pre-existing rows come from the table scan; rows this statement inserts are
    // invisible to that scan (a data-modifying CTE's writes aren't seen by sibling
    // reads), so they come from `ins` RETURNING instead — the same split the
    // single-name upsert uses to dodge the DO NOTHING/SELECT race.
    //
    // The scan compares `name::citext`, not the raw text: `name` is citext, so
    // ON CONFLICT collides case-insensitively. A capture differing from an
    // existing row only in case (`Monster` vs a stored `monster`) is then neither
    // inserted (conflict → DO NOTHING) nor found by a case-sensitive text scan —
    // it would fall through both branches and vanish from the map, silently
    // dropping the tag on every model and bundle category that needed it.
    let rows = sqlx::query!(
        r#"WITH input AS (SELECT DISTINCT unnest($1::text[]) AS name),
                ins AS (
                    INSERT INTO variant_tags (name)
                    SELECT name FROM input
                    ON CONFLICT (name) DO NOTHING
                    RETURNING id, name
                )
           SELECT id AS "id!", name::text AS "name!" FROM ins
           UNION ALL
           SELECT vt.id, vt.name::text FROM variant_tags vt
            WHERE vt.name IN (SELECT name::citext FROM input)
              AND vt.name NOT IN (SELECT name FROM ins)"#,
        &cleaned,
    )
    .fetch_all(&mut *tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.name.to_lowercase(), r.id))
        .collect())
}

/// Same, for the model `tags` vocabulary.
async fn upsert_tags_bulk(
    tx: &mut sqlx::PgConnection,
    names: &[String],
) -> Result<HashMap<String, Uuid>, ApiError> {
    let cleaned: Vec<String> = names
        .iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    if cleaned.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query!(
        r#"WITH input AS (SELECT DISTINCT unnest($1::text[]) AS name),
                ins AS (
                    INSERT INTO tags (name)
                    SELECT name FROM input
                    ON CONFLICT (name) DO NOTHING
                    RETURNING id, name
                )
           SELECT id AS "id!", name::text AS "name!" FROM ins
           UNION ALL
           SELECT t.id, t.name::text FROM tags t
            WHERE t.name IN (SELECT name::citext FROM input)
              AND t.name NOT IN (SELECT name FROM ins)"#,
        &cleaned,
    )
    .fetch_all(&mut *tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.name.to_lowercase(), r.id))
        .collect())
}

// ---------------------------------------------------------------------------
// the carve: execute a plan inside the commit transaction
// ---------------------------------------------------------------------------

/// Where a carve puts files that resolved no variant tags.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Untagged {
    /// Loose in the model's unsorted bucket (variant_id NULL) — a one-model carve.
    UnsortedBucket,
    /// The model's anonymous variant (empty tag set) — a bundle carve.
    AnonymousVariant,
}

/// Assign carved files: each tag set get-or-creates that variant of the model
/// (the existing merge-by-tag-set semantics). Files that resolved no variant
/// tags go either into the model's unsorted bucket or into its anonymous
/// variant (the empty-tag-set variant), per `untagged`.
///
/// Batched: variant tag names are pre-resolved (`vtag_ids`), variants and their
/// tag assignments are bulk-inserted, and every file moves in one UPDATE — so a
/// model with a dozen variants costs a handful of queries, not dozens. The
/// merge-by-tag-set rule is preserved in memory: planned variants that resolve
/// to the same tag set are folded together (two would otherwise collide on the
/// deferred `UNIQUE (model_id, tag_key)`), and for an existing member its
/// current variants are read once so a matching set reuses that variant.
async fn carve_variants(
    tx: &mut sqlx::PgConnection,
    model_id: Uuid,
    variants: &[PlanVariant],
    user_id: Uuid,
    untagged: Untagged,
    vtag_ids: &HashMap<String, Uuid>,
    model_is_new: bool,
) -> Result<(), ApiError> {
    // Fold planned variants by canonical (sorted, deduped) tag-id set. Set
    // equality is tag_key equality, so this is exactly the merge the per-variant
    // path got from the DB — done in memory, before anything is written.
    let mut by_set: HashMap<Vec<Uuid>, Vec<Uuid>> = HashMap::new();
    // A one-model carve leaves untagged files loose in the model's unsorted
    // bucket — they are just the model's own files. A bundle carve instead gives
    // each member its anonymous variant (empty tag set), a first-class sibling to
    // its tagged variants (so it renders a preview and reads as a variant, not as
    // leftovers the carve couldn't place).
    let mut unsorted_files: Vec<Uuid> = Vec::new();
    for planned in variants {
        let mut tag_ids: Vec<Uuid> = Vec::new();
        for name in &planned.tags {
            if let Some(id) = vtag_ids.get(&name.trim().to_lowercase())
                && !tag_ids.contains(id)
            {
                tag_ids.push(*id);
            }
        }
        tag_ids.sort();
        if tag_ids.is_empty() && untagged == Untagged::UnsortedBucket {
            unsorted_files.extend(planned.files.iter().copied());
            continue;
        }
        by_set
            .entry(tag_ids)
            .or_default()
            .extend(planned.files.iter().copied());
    }

    // A brand-new model has no variants to reuse; only an existing member (a
    // bundle re-drop landing on it) does, so read those once and never for a
    // model this commit just created.
    let existing: HashMap<Vec<Uuid>, Uuid> = if model_is_new {
        HashMap::new()
    } else {
        sqlx::query!(
            // `!`: NOT NULL on the preserved side of the LEFT JOIN (see exports.rs).
            r#"SELECT v.id as "id!",
                      coalesce(array_agg(a.tag_id) FILTER (WHERE a.tag_id IS NOT NULL), '{}')
                          AS "tags!: Vec<Uuid>"
               FROM model_variants v
               LEFT JOIN variant_tag_assignments a ON a.variant_id = v.id
               WHERE v.model_id = $1
               GROUP BY v.id"#,
            model_id,
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|r| {
            let mut tags = r.tags;
            tags.sort();
            tags.dedup();
            (tags, r.id)
        })
        .collect()
    };

    // Resolve every set to a variant id, collecting the new variants to create,
    // their tag assignments, and the file→variant moves — then flush each in one
    // statement.
    let mut new_variant_ids: Vec<Uuid> = Vec::new();
    let mut assign_variants: Vec<Uuid> = Vec::new();
    let mut assign_tags: Vec<Uuid> = Vec::new();
    let mut move_files: Vec<Uuid> = Vec::new();
    let mut move_variants: Vec<Uuid> = Vec::new();
    for (tag_ids, files) in by_set {
        let variant_id = match existing.get(&tag_ids) {
            Some(id) => *id,
            None => {
                let id = Uuid::new_v4();
                new_variant_ids.push(id);
                for tag_id in &tag_ids {
                    assign_variants.push(id);
                    assign_tags.push(*tag_id);
                }
                id
            }
        };
        for file in files {
            move_files.push(file);
            move_variants.push(variant_id);
        }
    }

    if !new_variant_ids.is_empty() {
        sqlx::query!(
            "INSERT INTO model_variants (id, model_id, created_by)
             SELECT unnest($1::uuid[]), $2, $3",
            &new_variant_ids,
            model_id,
            user_id,
        )
        .execute(&mut *tx)
        .await?;
    }
    if !assign_variants.is_empty() {
        // The tag_key trigger fires per assignment row, so each new variant lands
        // with its canonical key (an empty-tag anonymous variant inserts no
        // assignments and keeps its default '').
        sqlx::query!(
            "INSERT INTO variant_tag_assignments (variant_id, tag_id)
             SELECT unnest($1::uuid[]), unnest($2::uuid[])",
            &assign_variants,
            &assign_tags,
        )
        .execute(&mut *tx)
        .await?;
    }
    if !move_files.is_empty() {
        sqlx::query!(
            "UPDATE files SET variant_id = d.vid, import_id = NULL
             FROM (SELECT unnest($1::uuid[]) AS fid, unnest($2::uuid[]) AS vid) d
             WHERE files.id = d.fid",
            &move_files,
            &move_variants,
        )
        .execute(&mut *tx)
        .await?;
    }
    if !unsorted_files.is_empty() {
        sqlx::query!(
            "UPDATE files SET model_id = $1, import_id = NULL WHERE id = ANY($2::uuid[])",
            model_id,
            &unsorted_files,
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Move a model's imported image *files* into its image gallery. A photo that
/// shipped inside the archive — landed on the model itself or on one of its
/// variants by the carve — is a picture of the model, not something to download,
/// so it belongs in the `images` table rather than `files`. For each image blob
/// owned by the model or any of its variants: insert an `images` row owned by the
/// model (same blob, no new bytes), then drop the `files` row.
///
/// The shortest-named image becomes the model's primary if it hasn't got one
/// already — the same tie-break `model_preview_image` uses — so an included cover
/// shot becomes the model's picture instead of a rendered STL. A model that
/// already has a primary (a scraper cover, a promoted variant render) keeps it;
/// the adopted images just join the gallery. Deduped by (model, blob), so the
/// same picture sitting in two variants becomes one image.
async fn adopt_model_images(
    tx: &mut sqlx::PgConnection,
    model_id: Uuid,
    user_id: Uuid,
) -> Result<(), ApiError> {
    let images = sqlx::query!(
        // `!`: NOT NULL on the preserved side of the LEFT JOIN (see exports.rs).
        r#"SELECT f.id as "id!", f.blob_sha256 as "blob_sha256!", f.mime
           FROM files f
           LEFT JOIN model_variants v ON v.id = f.variant_id
           WHERE (f.model_id = $1 OR v.model_id = $1)
             AND f.mime LIKE 'image/%'
           ORDER BY length(f.filename), f.filename"#,
        model_id,
    )
    .fetch_all(&mut *tx)
    .await?;
    if images.is_empty() {
        return Ok(());
    }
    let mut file_ids: Vec<Uuid> = Vec::with_capacity(images.len());
    for img in &images {
        // Sequential inserts: once the first claims primary, `NOT EXISTS` turns
        // the rest false, so exactly one is primary — and none is if the model
        // already had one. `source_file_id` is left null on purpose: the file it
        // would point at is deleted just below (the FK would null it anyway), and
        // the explicit primary is what the preview picker keys on.
        sqlx::query!(
            r#"INSERT INTO images (blob_sha256, model_id, kind, mime, is_primary, created_by)
               SELECT $1, $2, 'imported', $3,
                      NOT EXISTS (SELECT 1 FROM images i WHERE i.model_id = $2 AND i.is_primary),
                      $4
               WHERE NOT EXISTS (
                   SELECT 1 FROM images i WHERE i.blob_sha256 = $1 AND i.model_id = $2
               )"#,
            img.blob_sha256,
            model_id,
            img.mime,
            user_id,
        )
        .execute(&mut *tx)
        .await?;
        file_ids.push(img.id);
    }
    sqlx::query!("DELETE FROM files WHERE id = ANY($1::uuid[])", &file_ids)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Stamp the import's metadata onto every model the carve created. `coalesce`
/// throughout: a blank field on the import page means "nothing to say", not
/// "erase what the carve worked out" — so a member model keeps the creator its
/// bundle gave it unless the import names a different one. Batched across all
/// created models, since a box set is bought once and its facts are identical
/// for each.
///
/// `description` says whether the typed description belongs on these models.
/// Under a bundle target it does not: purchase facts and tags are true of every
/// model in the box, but prose written about a box set is not a description of
/// each figure in it — the caller puts it on the bundle instead.
async fn apply_meta_bulk(
    tx: &mut sqlx::PgConnection,
    model_ids: &[Uuid],
    meta: &ImportMeta,
    user_id: Uuid,
    tag_ids: &HashMap<String, Uuid>,
    description: bool,
) -> Result<(), ApiError> {
    if model_ids.is_empty() {
        return Ok(());
    }
    sqlx::query!(
        r#"UPDATE models SET
             creator_id     = coalesce($2, creator_id),
             source_url     = coalesce($3, source_url),
             license        = coalesce($4, license),
             purchase_price = coalesce($5::float8::numeric(10,2), purchase_price),
             purchase_date  = coalesce($6, purchase_date),
             order_ref      = coalesce($7, order_ref)
           WHERE id = ANY($1::uuid[])"#,
        model_ids,
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
    // the ones typed on the import page both describe the model. Every created
    // model gets the same typed tags, so cross-join the two into one insert.
    let mtag_ids = mapped_ids(&meta.tags, tag_ids);
    if !mtag_ids.is_empty() {
        sqlx::query!(
            "INSERT INTO model_tags (model_id, tag_id)
             SELECT m.id, t.id
               FROM unnest($1::uuid[]) AS m(id) CROSS JOIN unnest($2::uuid[]) AS t(id)
             ON CONFLICT DO NOTHING",
            model_ids,
            &mtag_ids,
        )
        .execute(&mut *tx)
        .await?;
    }

    if let Some(body) = meta
        .description_md
        .as_deref()
        .map(str::trim)
        .filter(|_| description)
        .filter(|d| !d.is_empty())
    {
        sqlx::query!(
            "INSERT INTO model_description_revisions (model_id, body_md, created_by)
             SELECT unnest($1::uuid[]), $2, $3",
            model_ids,
            body,
            user_id,
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Put the import page's typed description on the bundle itself, where a bundle
/// target's description belongs.
///
/// Descriptions are immutable revisions (newest = current), so on a bundle that
/// already has one this inserts rather than overwrites — the old text stays
/// readable in the revision history. A blank field still means "nothing to say"
/// and writes nothing, which is what keeps a later expansion-pack drop from
/// blanking the box set's description.
async fn apply_bundle_description(
    tx: &mut sqlx::PgConnection,
    bundle_id: Uuid,
    meta: &ImportMeta,
    user_id: Uuid,
) -> Result<(), ApiError> {
    if let Some(body) = meta
        .description_md
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        sqlx::query!(
            "INSERT INTO bundle_description_revisions (bundle_id, body_md, created_by)
             VALUES ($1, $2, $3)",
            bundle_id,
            body,
            user_id,
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Resolve tag names to their (pre-upserted) ids via `map`, keyed by trimmed
/// lowercase name, dropping unknowns and duplicates.
fn mapped_ids(names: &[String], map: &HashMap<String, Uuid>) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = Vec::new();
    for name in names {
        if let Some(id) = map.get(&name.trim().to_lowercase())
            && !ids.contains(id)
        {
            ids.push(*id);
        }
    }
    ids
}

/// Additive model tagging — a carve never removes tags a model already has.
/// Names are pre-resolved via `tag_ids` (see [`upsert_tags_bulk`]), so this is
/// one insert regardless of how many tags the model carries.
async fn add_model_tags(
    tx: &mut sqlx::PgConnection,
    model_id: Uuid,
    names: &[String],
    tag_ids: &HashMap<String, Uuid>,
) -> Result<(), ApiError> {
    let ids = mapped_ids(names, tag_ids);
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query!(
        "INSERT INTO model_tags (model_id, tag_id)
         SELECT $1, unnest($2::uuid[]) ON CONFLICT DO NOTHING",
        model_id,
        &ids,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Carve a bundle-target plan: each planned (name + model-tag set) lands on a
/// member model — the one the user retargeted it to (`overrides`), else the one
/// [`match_member`] picks by name/alias + tag coverage, else a fresh member.
/// This is how a later DownloadAll_75mm drop lands its files on the models the
/// 32mm drop created. `overrides` is index-aligned to `plan.models`: `Some(id)`
/// forces that member, `None` forces a new one; an empty slice means auto-match
/// every model. Returns newly created model ids.
async fn carve_into_bundle(
    tx: &mut sqlx::PgConnection,
    bundle_id: Uuid,
    bundle_creator: Option<Uuid>,
    plan: &Plan,
    user_id: Uuid,
    overrides: &[Option<Uuid>],
    tags: &TagMaps,
) -> Result<Vec<Uuid>, ApiError> {
    if !overrides.is_empty() && overrides.len() != plan.models.len() {
        return Err(ApiError::BadRequest(
            "merge_targets must have one entry per planned model".into(),
        ));
    }
    let members = bundle_members(&mut *tx, bundle_id).await?;
    let member_ids: HashSet<Uuid> = members.iter().map(|m| m.id).collect();

    let mut reserved_slugs: HashSet<String> = HashSet::new();
    let mut created = Vec::new();
    for (i, planned) in plan.models.iter().enumerate() {
        // An explicit retarget choice wins; with none, fall back to the same
        // name/alias match the plan previewed.
        let chosen = if overrides.is_empty() {
            match_member(&members, planned).map(|m| m.id)
        } else {
            overrides[i]
        };
        if let Some(member_id) = chosen
            && !member_ids.contains(&member_id)
        {
            return Err(ApiError::BadRequest(
                "merge target is not a member of this bundle".into(),
            ));
        }
        // A fresh member has no variants to reuse; a matched one might, so
        // `carve_variants` reads its variants only when it isn't new.
        let model_is_new = chosen.is_none();
        let model_id = match chosen {
            Some(member_id) => member_id,
            None => {
                let slug = unique_member_slug(&mut *tx, &planned.name, &mut reserved_slugs).await?;
                let model_id: Uuid = sqlx::query_scalar!(
                    "INSERT INTO models (name, slug, creator_id, creator_ref, model_version, created_by)
                     VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
                    planned.name,
                    slug,
                    bundle_creator,
                    planned.creator_ref,
                    planned.model_version,
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
        add_model_tags(&mut *tx, model_id, &planned.tags, &tags.model).await?;
        carve_variants(
            &mut *tx,
            model_id,
            &planned.variants,
            user_id,
            Untagged::AnonymousVariant,
            &tags.variant,
            model_is_new,
        )
        .await?;
        // Images the carve placed on this member — whether it was freshly created
        // or matched to it by name — are pictures of it, not files to download.
        adopt_model_images(&mut *tx, model_id, user_id).await?;
    }

    // Record the bundle's sections (categories) from the carve's model tags, in
    // the order the folders present them. New ones append after any the bundle
    // already has (a later 75mm drop adds its sections at the end); ones already
    // recorded keep their position. The user can reorder/curate on the bundle page.
    let mut next_pos: i32 = sqlx::query_scalar!(
        r#"SELECT coalesce(max(position) + 1, 0) as "next!" FROM bundle_categories WHERE bundle_id = $1"#,
        bundle_id,
    )
    .fetch_one(&mut *tx)
    .await?;
    for name in &plan.model_tag_order {
        let Some(&tag_id) = tags.model.get(&name.trim().to_lowercase()) else {
            continue;
        };
        let inserted = sqlx::query!(
            "INSERT INTO bundle_categories (bundle_id, tag_id, position) VALUES ($1, $2, $3)
             ON CONFLICT (bundle_id, tag_id) DO NOTHING",
            bundle_id,
            tag_id,
            next_pos,
        )
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() > 0 {
            next_pos += 1;
        }
    }

    Ok(created)
}

/// Like `models::unique_slug`, but reading through the commit transaction and
/// aware of slugs this same carve has claimed but not yet made visible. Member
/// models get the same `name-token` shape as any other, so a later rename can
/// carry the token over.
async fn unique_member_slug(
    tx: &mut sqlx::PgConnection,
    name: &str,
    reserved: &mut HashSet<String>,
) -> Result<String, ApiError> {
    let base = slugify(name);
    loop {
        let candidate = format!("{base}-{}", slug_token());
        if reserved.contains(&candidate) {
            continue;
        }
        let clash = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM models WHERE slug = $1)",
            candidate
        )
        .fetch_one(&mut *tx)
        .await?
        .unwrap_or(false);
        if !clash {
            reserved.insert(candidate.clone());
            return Ok(candidate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: u128, name: &str, tags: &[&str], aliases: &[&str]) -> MemberRow {
        MemberRow {
            id: Uuid::from_u128(id),
            name: name.into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn planned(name: &str, tags: &[&str]) -> layout::PlanModel {
        layout::PlanModel {
            name: name.into(),
            creator_ref: None,
            model_version: None,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            file_count: 0,
            variants: Vec::new(),
            merge_target: None,
        }
    }

    #[test]
    fn matches_by_name_case_insensitively() {
        let members = [member(1, "Gold", &[], &[])];
        assert_eq!(
            match_member(&members, &planned("gold", &[])).map(|m| m.id),
            Some(members[0].id)
        );
    }

    #[test]
    fn matches_by_stored_alias() {
        // A past import renamed the member; the drop still spells the old name.
        let members = [member(1, "Gold_V2", &[], &["Gold"])];
        assert_eq!(
            match_member(&members, &planned("Gold", &[])).map(|m| m.id),
            Some(members[0].id),
            "an alias should resolve the member",
        );
    }

    #[test]
    fn requires_the_member_to_cover_captured_model_tags() {
        // Same name, but the plan captured a Heroes tag the member lacks: no
        // match, so a fresh member is made rather than polluting the wrong one.
        let members = [member(1, "Gold", &["Enemies"], &[])];
        assert!(match_member(&members, &planned("Gold", &["Heroes"])).is_none());
        // The member that does carry it matches (extra member tags are fine).
        let members = [member(1, "Gold", &["Heroes", "Set A"], &[])];
        assert!(match_member(&members, &planned("Gold", &["heroes"])).is_some());
    }

    #[test]
    fn a_different_name_does_not_match() {
        let members = [member(1, "Gold", &[], &["Golden"])];
        assert!(match_member(&members, &planned("Silver", &[])).is_none());
    }

    #[test]
    fn under_is_by_segment_not_by_string() {
        assert!(is_under("Loot", "Loot"));
        assert!(is_under("Loot/A", "Loot"));
        assert!(is_under("Loot/A/x", "Loot/A"));
        // A shared string prefix that stops mid-segment is not containment.
        assert!(!is_under("Loot/AB", "Loot/A"));
        assert!(!is_under("Loot", "Loot/A"));
    }

    #[test]
    fn common_prefix_stops_at_the_shared_folder() {
        let f = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // Siblings under one parent share it.
        assert_eq!(common_prefix(&f(&["Loot/A", "Loot/B"])), vec!["Loot"]);
        // Deeper agreement is kept.
        assert_eq!(
            common_prefix(&f(&["Loot/Set/A", "Loot/Set/B"])),
            vec!["Loot", "Set"]
        );
        // No shared folder → empty prefix, so paths are moved whole.
        assert_eq!(common_prefix(&f(&["A/x", "B/y"])), Vec::<&str>::new());
        // A string prefix that isn't a folder prefix doesn't count.
        assert_eq!(
            common_prefix(&f(&["Loot/A", "Loota/B"])),
            Vec::<&str>::new()
        );
    }
}
