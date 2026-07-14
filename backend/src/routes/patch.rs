//! Apply a scraped bundle patch onto an existing
//! bundle: match each patch model to a member model, then merge metadata under
//! the caller's chosen rules. Two steps, like every other write in here —
//! `preview` returns the match diff to confirm, `apply` writes it.
//!
//! Generic on purpose: it knows the `meshtrove.bundle-patch/1` schema, not Loot.
//! The upload is the scraper's zip — `{ patch.json, images/* }`.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    routing::post,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::tags::upsert_tag;
use crate::services::blobstore::BlobStore;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/bundles/{id}/patch/preview", post(preview))
        .route("/api/bundles/{id}/patch", post(apply))
        // The zip carries images; the store streams, so no body cap.
        .layer(DefaultBodyLimit::disable())
}

// ---------------------------------------------------------------------------
// The patch document — only the fields we read; everything else is ignored, so
// a richer patch from some future scraper still applies.
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct Patch {
    #[serde(default)]
    bundle: PatchBundle,
    #[serde(default)]
    models: Vec<PatchModel>,
}

#[derive(Deserialize, Default)]
struct PatchBundle {
    #[serde(default)]
    description_md: Option<String>,
    /// Candidate covers, primary first (relative paths into the zip).
    #[serde(default)]
    images: Vec<String>,
}

#[derive(Deserialize, Default)]
struct PatchModel {
    #[serde(default, rename = "match")]
    match_hint: MatchHint,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    /// Relative path into the zip, or null.
    #[serde(default)]
    image: Option<String>,
}

#[derive(Deserialize, Default)]
struct MatchHint {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
}

impl PatchModel {
    /// The label the UI and the options map key this model by: its rename target,
    /// else its match name.
    fn label(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.match_hint.name.clone())
            .unwrap_or_default()
    }
    /// Every spelling that could match a library model.
    fn match_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        for s in self
            .name
            .iter()
            .chain(self.match_hint.name.iter())
            .chain(self.match_hint.aliases.iter())
        {
            let k = normalize(s);
            if !k.is_empty() && !keys.contains(&k) {
                keys.push(k);
            }
        }
        keys
    }
}

/// Collapse camel case, spaces, underscores and hyphens to nothing, lowercased —
/// so the scraper's "Warrior Mummy" matches the carve's "WarriorMummy" folder
/// model without either side having to guess the other's spelling.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

// ---------------------------------------------------------------------------
// Reading the zip out of the multipart body.
// ---------------------------------------------------------------------------

struct Archive {
    patch: Patch,
    images: HashMap<String, Vec<u8>>,
}

async fn read_archive(mut multipart: Multipart) -> Result<Archive, ApiError> {
    let mut zip_bytes: Option<Vec<u8>> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("bad multipart body: {e}")))?
    {
        if field.name() == Some("file") {
            zip_bytes = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::BadRequest(format!("reading upload: {e}")))?
                    .to_vec(),
            );
        }
    }
    let bytes = zip_bytes.ok_or_else(|| ApiError::BadRequest("no file field in upload".into()))?;
    read_archive_from_bytes(bytes).await
}

// ---------------------------------------------------------------------------
// Matching: patch models against the bundle's members.
// ---------------------------------------------------------------------------

struct Member {
    id: Uuid,
    name: String,
    tags: Vec<String>,
}

async fn bundle_members(state: &AppState, bundle_id: Uuid) -> Result<Vec<Member>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT m.id, m.name,
                  coalesce((SELECT array_agg(t.name::text ORDER BY t.name) FROM model_tags mt
                            JOIN tags t ON t.id = mt.tag_id WHERE mt.model_id = m.id), '{}')
                      as "tags!: Vec<String>"
           FROM models m JOIN bundle_models bm ON bm.model_id = m.id
           WHERE bm.bundle_id = $1
           ORDER BY m.name"#,
        bundle_id,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Member {
            id: r.id,
            name: r.name,
            tags: r.tags,
        })
        .collect())
}

/// For each patch model, the member ids it could be. 0 = unmatched, 1 = matched,
/// >1 = ambiguous (the user picks).
fn resolve(patch: &Patch, members: &[Member]) -> Vec<Vec<Uuid>> {
    let mut by_key: HashMap<String, Vec<Uuid>> = HashMap::new();
    for m in members {
        by_key.entry(normalize(&m.name)).or_default().push(m.id);
    }
    patch
        .models
        .iter()
        .map(|pm| {
            let mut ids: Vec<Uuid> = Vec::new();
            for key in pm.match_keys() {
                for id in by_key.get(&key).into_iter().flatten() {
                    if !ids.contains(id) {
                        ids.push(*id);
                    }
                }
            }
            ids
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Preview.
// ---------------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
struct PatchPreview {
    bundle_has_description: bool,
    bundle_cover_count: usize,
    matched: Vec<MatchedRow>,
    ambiguous: Vec<AmbiguousRow>,
    /// Patch models that matched no member (nothing to apply them to).
    unmatched_patch: Vec<String>,
    /// Members the patch said nothing about (left untouched).
    unmatched_members: Vec<String>,
}

#[derive(Serialize, ToSchema)]
struct MatchedRow {
    patch_name: String,
    model_id: Uuid,
    model_name: String,
    /// Tags the patch would add that the model does not already have.
    add_tags: Vec<String>,
    has_image: bool,
}

#[derive(Serialize, ToSchema)]
struct AmbiguousRow {
    patch_name: String,
    candidates: Vec<Candidate>,
}

#[derive(Serialize, ToSchema)]
struct Candidate {
    id: Uuid,
    name: String,
}

async fn preview(
    State(state): State<AppState>,
    user: User,
    Path(bundle_id): Path<Uuid>,
    multipart: Multipart,
) -> Result<Json<PatchPreview>, ApiError> {
    let created_by = bundle_created_by(&state, bundle_id).await?;
    user.require_can_edit(created_by)?;

    let archive = read_archive(multipart).await?;
    let members = bundle_members(&state, bundle_id).await?;
    let resolution = resolve(&archive.patch, &members);
    let member_of = |id: Uuid| members.iter().find(|m| m.id == id);

    let mut matched = Vec::new();
    let mut ambiguous = Vec::new();
    let mut unmatched_patch = Vec::new();
    let mut claimed: std::collections::HashSet<Uuid> = std::collections::HashSet::new();

    for (pm, ids) in archive.patch.models.iter().zip(&resolution) {
        match ids.as_slice() {
            [] => unmatched_patch.push(pm.label()),
            [id] => {
                claimed.insert(*id);
                let member = member_of(*id);
                let have: std::collections::HashSet<String> = member
                    .map(|m| m.tags.iter().map(|t| t.to_lowercase()).collect())
                    .unwrap_or_default();
                let add_tags = pm
                    .tags
                    .iter()
                    .map(|t| t.to_lowercase())
                    .filter(|t| !have.contains(t))
                    .collect::<Vec<_>>();
                matched.push(MatchedRow {
                    patch_name: pm.label(),
                    model_id: *id,
                    model_name: member.map(|m| m.name.clone()).unwrap_or_default(),
                    add_tags,
                    has_image: pm
                        .image
                        .as_deref()
                        .is_some_and(|p| archive.images.contains_key(p)),
                });
            }
            many => {
                for id in many {
                    claimed.insert(*id);
                }
                ambiguous.push(AmbiguousRow {
                    patch_name: pm.label(),
                    candidates: many
                        .iter()
                        .filter_map(|id| member_of(*id))
                        .map(|m| Candidate {
                            id: m.id,
                            name: m.name.clone(),
                        })
                        .collect(),
                });
            }
        }
    }

    let unmatched_members = members
        .iter()
        .filter(|m| !claimed.contains(&m.id))
        .map(|m| m.name.clone())
        .collect();

    Ok(Json(PatchPreview {
        bundle_has_description: archive
            .patch
            .bundle
            .description_md
            .as_deref()
            .is_some_and(|d| !d.trim().is_empty()),
        bundle_cover_count: archive.patch.bundle.images.len(),
        matched,
        ambiguous,
        unmatched_patch,
        unmatched_members,
    }))
}

// ---------------------------------------------------------------------------
// Apply.
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum TagMode {
    #[default]
    Merge,
    Replace,
    Skip,
}

#[derive(Deserialize, Default, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum ImageMode {
    /// Drop the model's rendered previews and make the scraped image its picture.
    #[default]
    ReplaceGenerated,
    /// Keep the renders; add the scraped image and make it primary.
    Add,
    Skip,
}

#[derive(Deserialize, Default)]
struct ApplyOptions {
    #[serde(default)]
    rename_models: bool,
    #[serde(default)]
    model_tags: TagMode,
    #[serde(default)]
    model_images: ImageMode,
    #[serde(default)]
    bundle_cover: bool,
    #[serde(default)]
    bundle_description: bool,
    /// Resolves ambiguous rows and adopts unmatched ones: patch model label →
    /// the member id to apply it to. Overrides the automatic single match too.
    #[serde(default)]
    matches: HashMap<String, Uuid>,
}

#[derive(Serialize, ToSchema)]
struct ApplyResult {
    models_updated: usize,
    images_added: usize,
    tags_added: usize,
}

/// A blob already put into the store, ready to become an image row.
struct StagedImage {
    sha256: String,
    size: i64,
    mime: String,
}

async fn stage_image(
    state: &AppState,
    images: &HashMap<String, Vec<u8>>,
    path: &str,
) -> Result<Option<StagedImage>, ApiError> {
    let Some(bytes) = images.get(path) else {
        return Ok(None);
    };
    let mime = mime_guess::from_path(path)
        .first()
        .map(|m| m.to_string())
        .unwrap_or_else(|| "image/png".into());
    let bytes = bytes.clone();
    let stream = futures::stream::once(async move { Ok(bytes::Bytes::from(bytes)) });
    let blob = state.store.put(Box::pin(stream)).await?;
    Ok(Some(StagedImage {
        sha256: blob.sha256,
        size: blob.size,
        mime,
    }))
}

async fn apply(
    State(state): State<AppState>,
    user: User,
    Path(bundle_id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<ApplyResult>, ApiError> {
    let created_by = bundle_created_by(&state, bundle_id).await?;
    user.require_can_edit(created_by)?;

    // options come as a text field alongside the file; read both.
    let mut options = ApplyOptions::default();
    let mut zip_bytes: Option<Vec<u8>> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("bad multipart body: {e}")))?
    {
        match field.name() {
            Some("options") => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| ApiError::BadRequest(format!("bad options field: {e}")))?;
                options = serde_json::from_str(&text)
                    .map_err(|e| ApiError::BadRequest(format!("bad options: {e}")))?;
            }
            Some("file") => {
                zip_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("reading upload: {e}")))?
                        .to_vec(),
                );
            }
            _ => {}
        }
    }
    let archive = read_archive_from_bytes(
        zip_bytes.ok_or_else(|| ApiError::BadRequest("no file field in upload".into()))?,
    )
    .await?;

    let members = bundle_members(&state, bundle_id).await?;
    let resolution = resolve(&archive.patch, &members);

    // Which member each patch model applies to: the automatic single match, then
    // the caller's overrides (which also adopt ambiguous/unmatched rows).
    let mut targets: Vec<(usize, Uuid)> = Vec::new();
    for (idx, (pm, ids)) in archive.patch.models.iter().zip(&resolution).enumerate() {
        let chosen = options
            .matches
            .get(&pm.label())
            .copied()
            .or(if ids.len() == 1 { Some(ids[0]) } else { None });
        if let Some(model_id) = chosen {
            targets.push((idx, model_id));
        }
    }

    // Put every image blob before opening the transaction — a blob put is async
    // filesystem work, and a rollback must not have to un-write the store.
    let mut model_images: HashMap<usize, StagedImage> = HashMap::new();
    if options.model_images != ImageMode::Skip {
        for (idx, _) in &targets {
            if let Some(path) = archive.patch.models[*idx].image.as_deref()
                && let Some(staged) = stage_image(&state, &archive.images, path).await?
            {
                model_images.insert(*idx, staged);
            }
        }
    }
    let mut cover_images: Vec<StagedImage> = Vec::new();
    if options.bundle_cover {
        for path in &archive.patch.bundle.images {
            if let Some(staged) = stage_image(&state, &archive.images, path).await? {
                cover_images.push(staged);
            }
        }
    }

    let mut result = ApplyResult {
        models_updated: 0,
        images_added: 0,
        tags_added: 0,
    };
    let mut tx = state.db.begin().await?;

    // ---- bundle-level ----
    if options.bundle_description
        && let Some(body) = archive
            .patch
            .bundle
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
            user.id,
        )
        .execute(&mut *tx)
        .await?;
    }
    if options.bundle_cover && !cover_images.is_empty() {
        // First cover becomes primary; the rest are added alongside.
        sqlx::query!(
            "UPDATE images SET is_primary = false WHERE bundle_id = $1 AND is_primary",
            bundle_id,
        )
        .execute(&mut *tx)
        .await?;
        for (n, img) in cover_images.iter().enumerate() {
            if insert_image(&mut tx, ImageOwner::Bundle(bundle_id), img, n == 0, user.id).await? {
                result.images_added += 1;
            }
        }
    }

    // ---- per model ----
    for (idx, model_id) in &targets {
        let pm = &archive.patch.models[*idx];
        let mut touched = false;

        if options.rename_models
            && let Some(name) = pm.name.as_deref().map(str::trim).filter(|n| !n.is_empty())
        {
            sqlx::query!("UPDATE models SET name = $2 WHERE id = $1", model_id, name)
                .execute(&mut *tx)
                .await?;
            touched = true;
        }

        if options.model_tags != TagMode::Skip && !pm.tags.is_empty() {
            if options.model_tags == TagMode::Replace {
                sqlx::query!("DELETE FROM model_tags WHERE model_id = $1", model_id)
                    .execute(&mut *tx)
                    .await?;
            }
            for name in &pm.tags {
                let tag = upsert_tag(&state, name).await?;
                let added = sqlx::query!(
                    "INSERT INTO model_tags (model_id, tag_id) VALUES ($1, $2)
                     ON CONFLICT DO NOTHING",
                    model_id,
                    tag.id,
                )
                .execute(&mut *tx)
                .await?;
                result.tags_added += added.rows_affected() as usize;
            }
            touched = true;
        }

        if let Some(img) = model_images.get(idx) {
            if options.model_images == ImageMode::ReplaceGenerated {
                // The auto previews live on the model's variants (and, rarely, the
                // model). Drop them so the scraped image is what shows.
                sqlx::query!(
                    "DELETE FROM images WHERE kind = 'rendered' AND (model_id = $1
                       OR variant_id IN (SELECT id FROM model_variants WHERE model_id = $1))",
                    model_id,
                )
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query!(
                "UPDATE images SET is_primary = false WHERE model_id = $1 AND is_primary",
                model_id,
            )
            .execute(&mut *tx)
            .await?;
            if insert_image(&mut tx, ImageOwner::Model(*model_id), img, true, user.id).await? {
                result.images_added += 1;
            }
            touched = true;
        }

        if touched {
            result.models_updated += 1;
        }
    }

    tx.commit().await?;
    tracing::info!(bundle = %bundle_id, ?result.models_updated, "bundle patch applied");
    Ok(Json(result))
}

enum ImageOwner {
    Model(Uuid),
    Bundle(Uuid),
}

/// Insert an image for an owner from an already-staged blob, deduped: if the
/// owner already carries that exact blob, only its primary flag is touched.
/// Returns whether a new row was inserted.
async fn insert_image(
    tx: &mut sqlx::PgConnection,
    owner: ImageOwner,
    img: &StagedImage,
    primary: bool,
    user_id: Uuid,
) -> Result<bool, ApiError> {
    let (model_id, bundle_id) = match owner {
        ImageOwner::Model(id) => (Some(id), None),
        ImageOwner::Bundle(id) => (None, Some(id)),
    };
    sqlx::query!(
        "INSERT INTO blobs (sha256, size) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        img.sha256,
        img.size,
    )
    .execute(&mut *tx)
    .await?;
    let inserted = sqlx::query!(
        r#"INSERT INTO images (blob_sha256, model_id, bundle_id, kind, mime, is_primary, created_by)
           SELECT $1, $2, $3, 'imported', $4, $5, $6
           WHERE NOT EXISTS (
               SELECT 1 FROM images i
               WHERE i.blob_sha256 = $1
                 AND ((i.model_id = $2 AND $2 IS NOT NULL)
                   OR (i.bundle_id = $3 AND $3 IS NOT NULL)))"#,
        img.sha256,
        model_id,
        bundle_id,
        img.mime,
        primary,
        user_id,
    )
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() == 0 && primary {
        // Already present — just make sure it is the primary one.
        sqlx::query!(
            "UPDATE images SET is_primary = true
             WHERE blob_sha256 = $1
               AND ((model_id = $2 AND $2 IS NOT NULL) OR (bundle_id = $3 AND $3 IS NOT NULL))",
            img.sha256,
            model_id,
            bundle_id,
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(inserted.rows_affected() > 0)
}

async fn read_archive_from_bytes(bytes: Vec<u8>) -> Result<Archive, ApiError> {
    tokio::task::spawn_blocking(move || {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| ApiError::BadRequest(format!("not a zip: {e}")))?;
        let mut patch_json: Option<Vec<u8>> = None;
        let mut images = HashMap::new();
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|e| ApiError::BadRequest(format!("bad zip entry: {e}")))?;
            let Some(name) = entry
                .enclosed_name()
                .map(|p| p.to_string_lossy().to_string())
            else {
                continue;
            };
            if !entry.is_file() {
                continue;
            }
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| ApiError::BadRequest(format!("reading {name}: {e}")))?;
            if name == "patch.json" || name.ends_with("/patch.json") {
                patch_json = Some(buf);
            } else {
                images.insert(name, buf);
            }
        }
        let patch_json =
            patch_json.ok_or_else(|| ApiError::BadRequest("no patch.json in the zip".into()))?;
        let patch: Patch = serde_json::from_slice(&patch_json)
            .map_err(|e| ApiError::BadRequest(format!("bad patch.json: {e}")))?;
        Ok(Archive { patch, images })
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
}

async fn bundle_created_by(state: &AppState, bundle_id: Uuid) -> Result<Uuid, ApiError> {
    sqlx::query_scalar!("SELECT created_by FROM bundles WHERE id = $1", bundle_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)
}
