//! Restore a MeshTrove export that was dropped into an import.
//!
//! A dropped zip is stored as an archive blob and, if it carries a
//! `manifest.json`, the upload flags the import as an export (`files.rs`) instead
//! of queueing the usual unpack. These two endpoints work off that already-stored
//! blob — `preview` reads just the manifest (cheap; flags entities already
//! present), `commit` streams the blobs into the store and restores the entities,
//! then discards the import.
//!
//! Building an export archive is the other half of the same feature; it lives in
//! `routes/exports.rs` because it is an asynchronous, job-backed artifact.

use std::collections::HashSet;

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::imports::import_created_by;
use crate::services::transfer::{
    self, CustomFieldMapping, RestoreOptions, RestoreSummary, suggest_mapping,
};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/imports/{id}/restore/preview", get(restore_preview))
        .route("/api/imports/{id}/restore/commit", post(restore_commit))
}

#[derive(Serialize, ToSchema)]
struct RestorePreview {
    schema: String,
    exported_at: DateTime<Utc>,
    models: Vec<EntityRow>,
    bundles: Vec<EntityRow>,
    /// Every custom field the archive carries, with what this instance would do
    /// with it by default. The UI turns each row into a combo box.
    custom_fields: Vec<CustomFieldRow>,
    /// This instance's own vocabulary, to populate that combo box.
    local_custom_fields: Vec<LocalCustomField>,
    blob_count: usize,
    total_size: i64,
}

/// One exported custom field definition, as the restore screen shows it.
#[derive(Serialize, ToSchema)]
struct CustomFieldRow {
    /// The manifest-local id — the key of the `custom_fields` mapping on commit.
    id: Uuid,
    key: String,
    name: String,
    kind: String,
    applies_to_models: bool,
    applies_to_bundles: bool,
    visibility: String,
    /// How many values across the archive would be written under it.
    value_count: usize,
    /// The local field this instance would adopt by default (same key), or null
    /// when it would create the field instead.
    suggested_field_id: Option<Uuid>,
}

#[derive(Serialize, ToSchema)]
struct LocalCustomField {
    id: Uuid,
    key: String,
    name: String,
    kind: String,
    applies_to_models: bool,
    applies_to_bundles: bool,
}

#[derive(Serialize, ToSchema)]
struct EntityRow {
    /// The manifest-local id — pass it back in `fresh` to force a fresh copy.
    id: Uuid,
    name: String,
    slug: String,
    /// An entity with this slug already exists here; it is skipped unless the
    /// user asks for a fresh copy.
    exists: bool,
    /// Member count, for bundles.
    #[serde(skip_serializing_if = "Option::is_none")]
    members: Option<usize>,
}

async fn restore_preview(
    State(state): State<AppState>,
    user: User,
    Path(import_id): Path<Uuid>,
) -> Result<Json<RestorePreview>, ApiError> {
    user.require_editor()?;
    let (archive_sha, _) = import_archive(&state, import_id).await?;
    let manifest = transfer::read_manifest_from_blob(&state.store, &archive_sha)
        .await?
        .ok_or_else(|| ApiError::BadRequest("this import is not a MeshTrove export".into()))?;

    // Which slugs already exist here.
    let model_slugs: Vec<String> = manifest.models.iter().map(|m| m.slug.clone()).collect();
    let existing_models: HashSet<String> =
        sqlx::query_scalar!("SELECT slug FROM models WHERE slug = ANY($1)", &model_slugs)
            .fetch_all(&state.db)
            .await?
            .into_iter()
            .collect();
    let bundle_slugs: Vec<String> = manifest.bundles.iter().map(|b| b.slug.clone()).collect();
    let existing_bundles: HashSet<String> = sqlx::query_scalar!(
        "SELECT slug FROM bundles WHERE slug = ANY($1)",
        &bundle_slugs
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();

    let models = manifest
        .models
        .iter()
        .map(|m| EntityRow {
            id: m.id,
            name: m.name.clone(),
            slug: m.slug.clone(),
            exists: existing_models.contains(&m.slug),
            members: None,
        })
        .collect();
    let bundles = manifest
        .bundles
        .iter()
        .map(|b| EntityRow {
            id: b.id,
            name: b.name.clone(),
            slug: b.slug.clone(),
            exists: existing_bundles.contains(&b.slug),
            members: Some(b.member_ids.len()),
        })
        .collect();

    // The archive's vocabulary, each row carrying the default this instance
    // would take and how much rides on it.
    let local = crate::routes::custom_fields::all_fields(&mut *state.db.acquire().await?).await?;
    let local_by_key: std::collections::HashMap<String, Uuid> =
        local.iter().map(|f| (f.key.to_lowercase(), f.id)).collect();
    let mut value_counts: std::collections::HashMap<Uuid, usize> = std::collections::HashMap::new();
    for values in manifest
        .models
        .iter()
        .map(|m| &m.custom_fields)
        .chain(manifest.bundles.iter().map(|b| &b.custom_fields))
    {
        for v in values {
            *value_counts.entry(v.field_id).or_default() += 1;
        }
    }
    let custom_fields = manifest
        .custom_fields
        .iter()
        .map(|d| CustomFieldRow {
            id: d.id,
            key: d.key.clone(),
            name: d.name.clone(),
            kind: d.kind.clone(),
            applies_to_models: d.applies_to_models,
            applies_to_bundles: d.applies_to_bundles,
            visibility: d.visibility.clone(),
            value_count: value_counts.get(&d.id).copied().unwrap_or(0),
            suggested_field_id: match suggest_mapping(d, &local_by_key) {
                CustomFieldMapping::Existing { field_id } => Some(field_id),
                _ => None,
            },
        })
        .collect();
    let local_custom_fields = local
        .into_iter()
        .map(|f| LocalCustomField {
            id: f.id,
            key: f.key,
            name: f.name,
            kind: serde_json::to_value(f.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
            applies_to_models: f.applies_to_models,
            applies_to_bundles: f.applies_to_bundles,
        })
        .collect();

    Ok(Json(RestorePreview {
        schema: manifest.schema.clone(),
        custom_fields,
        local_custom_fields,
        exported_at: manifest.exported_at,
        models,
        bundles,
        blob_count: manifest.blobs.len(),
        total_size: manifest.blobs.iter().map(|b| b.size).sum(),
    }))
}

#[derive(Deserialize)]
struct RestoreBody {
    /// Manifest-local ids of entities to import as a fresh copy even though one
    /// with the same slug already exists.
    #[serde(default)]
    fresh: Vec<Uuid>,
    /// What to do with each custom field the archive carries, by its
    /// manifest-local id: `{"action":"skip"}`, `{"action":"create"}`, or
    /// `{"action":"existing","field_id":"…"}`. A field left out takes the same
    /// default the preview suggested.
    #[serde(default)]
    custom_fields: std::collections::HashMap<Uuid, CustomFieldMapping>,
}

async fn restore_commit(
    State(state): State<AppState>,
    user: User,
    Path(import_id): Path<Uuid>,
    Json(body): Json<RestoreBody>,
) -> Result<Json<RestoreSummary>, ApiError> {
    user.require_can_edit(import_created_by(&state, import_id).await?)?;

    let (archive_sha, _) = import_archive(&state, import_id).await?;
    let manifest = transfer::read_manifest_from_blob(&state.store, &archive_sha)
        .await?
        .ok_or_else(|| ApiError::BadRequest("this import is not a MeshTrove export".into()))?;

    // Stream the archive's blobs into the store, then write the entities. Both
    // halves are logged with their timings: a restore is minutes of work on a
    // large archive, and until it finished there was nothing at all in the log to
    // say whether it was moving.
    tracing::info!(
        blobs = manifest.blobs.len(),
        bytes = manifest.blobs.iter().map(|b| b.size).sum::<i64>(),
        models = manifest.models.len(),
        bundles = manifest.bundles.len(),
        "restore: staging blobs"
    );
    let started = std::time::Instant::now();
    let staged = transfer::stage_blobs(&state.store, &archive_sha, &manifest).await?;
    tracing::info!(
        staged = staged.staged,
        already_held = staged.skipped,
        bytes = staged.bytes,
        elapsed_ms = started.elapsed().as_millis(),
        "restore: blobs staged"
    );

    let options = RestoreOptions {
        fresh: body.fresh.into_iter().collect(),
        custom_fields: body.custom_fields,
    };
    let entities = std::time::Instant::now();
    let summary = transfer::restore(&state, &user, &manifest, &options).await?;
    tracing::info!(
        models = summary.models_created,
        bundles = summary.bundles_created,
        files = summary.files,
        images = summary.images,
        custom_fields = summary.custom_fields_created,
        custom_field_values = summary.custom_field_values,
        elapsed_ms = entities.elapsed().as_millis(),
        total_ms = started.elapsed().as_millis(),
        "restore: complete"
    );

    // The staging import (and its now-redundant archive blob) has served its
    // purpose; drop it. The archive blob stays in the store for orphan GC.
    sqlx::query!("DELETE FROM imports WHERE id = $1", import_id)
        .execute(&state.db)
        .await?;

    Ok(Json(summary))
}

/// The archive blob an import is holding: its sha and filename.
async fn import_archive(state: &AppState, import_id: Uuid) -> Result<(String, String), ApiError> {
    sqlx::query!(
        "SELECT blob_sha256, filename FROM files
         WHERE import_id = $1 AND kind = 'archive'::file_kind
         ORDER BY created_at DESC LIMIT 1",
        import_id,
    )
    .fetch_optional(&state.db)
    .await?
    .map(|r| (r.blob_sha256, r.filename))
    .ok_or(ApiError::NotFound)
}
