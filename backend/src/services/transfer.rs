//! Export and import a model or a bundle as a self-describing archive.
//!
//! An archive is a zip with two layers. The readable layer mirrors the
//! collection on disk so it can be browsed without ever importing: a model
//! export puts the model at the root as a CamelCase folder, and a bundle export
//! nests its members *inside* the bundle, grouped by the bundle's category tabs
//! (`Undead-Horde/Heroes/WarriorMummy/variants/32mm/warrior.stl`). The
//! authoritative layer is `manifest.json`: every entity with its metadata, flat,
//! cross-referenced by id, and for each file the `archive_path` where its bytes
//! sit. Import trusts the manifest and never parses the tree, so the tree is free
//! to be lossy (colliding readable paths get a ` (2)` suffix in the archive path
//! only) and to nest members without confusing the restore.
//!
//! `gather_model` / `gather_bundle` build a manifest (plus the readable-only text
//! files); `restore` writes a manifest back, remapping every id and either
//! skipping or fresh-copying entities that are already present. Blob bytes are
//! streamed into the store by the route layer before `restore` runs, mirroring
//! how `patch.rs` stages images before opening its transaction.

use std::collections::{HashMap, HashSet};
use std::path::{Path as FsPath, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::tags::upsert_tag;
use crate::services::blobstore::{BlobStore, FsBlobStore};
use crate::state::AppState;

pub const SCHEMA: &str = "meshtrove.export/1";

// ---------------------------------------------------------------------------
// The manifest — the authoritative description of what the archive holds.
// Optional collections default so a manifest that predates a field still loads.
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub exported_at: DateTime<Utc>,
    #[serde(default)]
    pub creators: Vec<Creator>,
    /// Model/bundle tag vocabulary (the names; ids are re-minted on import).
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub variant_tags: Vec<VariantTag>,
    #[serde(default)]
    pub models: Vec<Model>,
    #[serde(default)]
    pub bundles: Vec<Bundle>,
    /// Every blob the file/image entries reference, with its size.
    #[serde(default)]
    pub blobs: Vec<Blob>,
}

#[derive(Serialize, Deserialize)]
pub struct Creator {
    /// The exporting instance's id — a manifest-local key the entities point at.
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct VariantTag {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Blob {
    pub sha256: String,
    pub size: i64,
}

#[derive(Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub creator_id: Option<Uuid>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub purchase_price: Option<f64>,
    #[serde(default)]
    pub purchase_date: Option<NaiveDate>,
    #[serde(default)]
    pub order_ref: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Description revisions, oldest first (newest = current).
    #[serde(default)]
    pub descriptions: Vec<Description>,
    #[serde(default)]
    pub variants: Vec<Variant>,
    /// Model-level files (documents): not owned by any variant.
    #[serde(default)]
    pub files: Vec<File>,
    #[serde(default)]
    pub images: Vec<Image>,
    #[serde(default)]
    pub source_archive: Option<SourceArchive>,
}

#[derive(Serialize, Deserialize)]
pub struct Variant {
    pub id: Uuid,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub print_notes: Option<String>,
    #[serde(default)]
    pub derived_from_variant_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub files: Vec<File>,
    #[serde(default)]
    pub images: Vec<Image>,
}

#[derive(Serialize, Deserialize)]
pub struct Description {
    pub body_md: String,
    #[serde(default)]
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct File {
    /// Manifest-local key so an image can point at its source file.
    pub id: Uuid,
    pub blob_sha256: String,
    #[serde(default)]
    pub path: String,
    pub filename: String,
    #[serde(default)]
    pub mime: Option<String>,
    pub kind: String,
    pub created_at: DateTime<Utc>,
    /// Where the bytes live inside the zip.
    pub archive_path: String,
}

#[derive(Serialize, Deserialize)]
pub struct Image {
    pub blob_sha256: String,
    pub kind: String,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub height: Option<i32>,
    pub is_primary: bool,
    pub sort_order: i32,
    #[serde(default)]
    pub renderer: Option<String>,
    #[serde(default)]
    pub renderer_config: Option<serde_json::Value>,
    /// The file this image was rendered from, by manifest-local file id.
    #[serde(default)]
    pub source_file_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub archive_path: String,
}

#[derive(Serialize, Deserialize)]
pub struct SourceArchive {
    pub filename: String,
    pub sha256: String,
    pub size: i64,
    pub imported_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct Bundle {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub creator_id: Option<Uuid>,
    #[serde(default)]
    pub source_url: Option<String>,
    pub kind: String,
    pub name_autogenerated: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub descriptions: Vec<Description>,
    /// Member models, by manifest-local model id.
    #[serde(default)]
    pub member_ids: Vec<Uuid>,
    /// Child bundles, by manifest-local bundle id.
    #[serde(default)]
    pub child_ids: Vec<Uuid>,
    /// Ordered category tabs: tag name + position.
    #[serde(default)]
    pub categories: Vec<Category>,
    #[serde(default)]
    pub files: Vec<File>,
    #[serde(default)]
    pub images: Vec<Image>,
    #[serde(default)]
    pub source_archive: Option<SourceArchive>,
}

#[derive(Serialize, Deserialize)]
pub struct Category {
    pub tag: String,
    pub position: i32,
}

/// A manifest plus the readable-only text files (description.md, README) that
/// accompany it in the zip but are not part of the authoritative layer.
pub struct Export {
    pub manifest: Manifest,
    pub texts: Vec<(String, Vec<u8>)>,
}

// ---------------------------------------------------------------------------
// Readable-path helpers: turn logical names into safe, unique archive paths.
// ---------------------------------------------------------------------------

/// One path segment made safe for a zip: anything outside a conservative set
/// becomes `_`, and a segment that would be empty (or `.`/`..`) collapses to `_`.
fn seg(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '(' | ')' | '[' | ']') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() || trimmed == ".." {
        "_".into()
    } else {
        trimmed.to_string()
    }
}

/// A name as a single CamelCase folder: `Warrior Mummy` -> `WarriorMummy`,
/// `knight-errant` -> `KnightErrant`. Internal casing is kept (so an acronym
/// survives), only the first letter of each word is forced upper.
fn camel(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for word in name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() {
        "_".into()
    } else {
        seg(&out)
    }
}

/// A logical `path` (which may itself carry `/`-separated subfolders) sanitised
/// segment by segment; empty segments are dropped.
fn rel_path(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .filter(|p| !p.is_empty())
        .map(seg)
        .collect()
}

/// The readable folder name for a variant: its tags sorted and joined, or
/// `_base` for the anonymous (no-tag) variant.
fn variant_dir(tags: &[String]) -> String {
    if tags.is_empty() {
        return "_base".into();
    }
    let mut parts: Vec<String> = tags.iter().map(|t| seg(t)).collect();
    parts.sort();
    seg(&parts.join("-"))
}

/// A file extension guessed from a mime type, for naming images that have no
/// filename of their own.
fn ext_for(mime: Option<&str>) -> String {
    match mime {
        Some("image/png") => "png",
        Some("image/jpeg") => "jpg",
        Some("image/webp") => "webp",
        Some("image/gif") => "gif",
        Some(m) => return seg(m.rsplit('/').next().unwrap_or("img")).to_lowercase(),
        None => "img",
    }
    .to_string()
}

/// Assigns unique archive paths and remembers, per (readable path, sha), the one
/// already assigned — so a file duplicated at the *same* readable location writes
/// once, while the same bytes at a different location write again (the readable
/// tree wins over dedup, by design).
#[derive(Default)]
struct PathAssigner {
    used: HashSet<String>,
    seen: HashMap<(String, String), String>,
}

impl PathAssigner {
    /// Returns the archive path these bytes occupy at this readable location.
    fn assign(&mut self, desired: &str, sha: &str) -> String {
        let key = (desired.to_string(), sha.to_string());
        if let Some(existing) = self.seen.get(&key) {
            return existing.clone();
        }
        let path = self.unique(desired);
        self.used.insert(path.clone());
        self.seen.insert(key, path.clone());
        path
    }

    /// A path not yet used, inserting ` (2)`, ` (3)`, … before the extension.
    fn unique(&self, desired: &str) -> String {
        if !self.used.contains(desired) {
            return desired.to_string();
        }
        let (stem, ext) = match desired.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() && !s.ends_with('/') => (s, format!(".{e}")),
            _ => (desired, String::new()),
        };
        let mut n = 2;
        loop {
            let candidate = format!("{stem} ({n}){ext}");
            if !self.used.contains(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// gather: read a model or a bundle into a manifest + readable tree.
// ---------------------------------------------------------------------------

/// Where each entity's folder sits in the readable tree.
#[derive(Default)]
struct Placement {
    model_base: HashMap<Uuid, String>,
    bundle_base: HashMap<Uuid, String>,
}

/// Which variants of a model to export. A variant passes if it carries every
/// `include` tag and none of the `exclude` tags (case-insensitive) — so
/// `exclude = ["supported"]` is "unsupported only", the anonymous variant
/// included. Empty include = no positive constraint.
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct VariantFilter {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl VariantFilter {
    fn matches(&self, tags: &[String]) -> bool {
        let have: HashSet<String> = tags.iter().map(|t| t.to_lowercase()).collect();
        self.include
            .iter()
            .all(|t| have.contains(&t.to_lowercase()))
            && self
                .exclude
                .iter()
                .all(|t| !have.contains(&t.to_lowercase()))
    }
}

/// What an export gathers: an optional bundle (for nesting and its own
/// metadata), the models to include, and which of their variants.
#[derive(Serialize, Deserialize)]
pub struct ExportSpec {
    #[serde(default)]
    pub bundle_id: Option<Uuid>,
    #[serde(default)]
    pub model_ids: Vec<Uuid>,
    #[serde(default)]
    pub filter: VariantFilter,
}

/// Gather an export from its spec: the chosen models (their filtered variants),
/// and — if a bundle is named — that bundle's metadata with the members nested
/// inside it under its category tabs.
pub async fn gather_export(
    db: &PgPool,
    spec: &ExportSpec,
    exported_at: DateTime<Utc>,
) -> Result<Export, ApiError> {
    let placement = match spec.bundle_id {
        Some(bundle_id) => bundle_placement(db, bundle_id, &spec.model_ids).await?,
        None => {
            // Each selected model as a CamelCase folder at the archive root.
            let mut p = Placement::default();
            for r in sqlx::query!(
                "SELECT id, name FROM models WHERE id = ANY($1)",
                &spec.model_ids
            )
            .fetch_all(db)
            .await?
            {
                p.model_base.insert(r.id, camel(&r.name));
            }
            p
        }
    };
    let bundle_ids: Vec<Uuid> = spec.bundle_id.into_iter().collect();
    gather_core(
        db,
        &spec.model_ids,
        &bundle_ids,
        &placement,
        &spec.filter,
        exported_at,
    )
    .await
}

/// Placement for a bundle export: the bundle folder holds each selected member,
/// nested under its first matching category tab (or directly under the bundle).
async fn bundle_placement(
    db: &PgPool,
    bundle_id: Uuid,
    members: &[Uuid],
) -> Result<Placement, ApiError> {
    let bname = sqlx::query_scalar!("SELECT name FROM bundles WHERE id = $1", bundle_id)
        .fetch_optional(db)
        .await?
        .ok_or(ApiError::NotFound)?;
    let base = camel(&bname);
    let mut p = Placement::default();
    p.bundle_base.insert(bundle_id, base.clone());

    let cats: Vec<String> = sqlx::query_scalar!(
        r#"SELECT t.name::text as "name!" FROM bundle_categories bc
           JOIN tags t ON t.id = bc.tag_id
           WHERE bc.bundle_id = $1 ORDER BY bc.position"#,
        bundle_id
    )
    .fetch_all(db)
    .await?;

    let names: HashMap<Uuid, String> =
        sqlx::query!("SELECT id, name FROM models WHERE id = ANY($1)", members)
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|r| (r.id, r.name))
            .collect();
    let mut mtags: HashMap<Uuid, HashSet<String>> = HashMap::new();
    for r in sqlx::query!(
        r#"SELECT mt.model_id, t.name::text as "name!" FROM model_tags mt
           JOIN tags t ON t.id = mt.tag_id WHERE mt.model_id = ANY($1)"#,
        members
    )
    .fetch_all(db)
    .await?
    {
        mtags
            .entry(r.model_id)
            .or_default()
            .insert(r.name.to_lowercase());
    }

    for m in members {
        let empty = HashSet::new();
        let tags = mtags.get(m).unwrap_or(&empty);
        let category = cats.iter().find(|c| tags.contains(&c.to_lowercase()));
        let name = names.get(m).map_or("", |s| s);
        let model_base = match category {
            Some(c) => format!("{base}/{}/{}", seg(c), camel(name)),
            None => format!("{base}/{}", camel(name)),
        };
        p.model_base.insert(*m, model_base);
    }
    Ok(p)
}

/// The shared body: read every model and bundle in the set into the manifest,
/// laying files out under the folders `placement` assigns, keeping only the
/// variants that pass `filter`.
async fn gather_core(
    db: &PgPool,
    model_ids: &[Uuid],
    bundle_ids: &[Uuid],
    placement: &Placement,
    filter: &VariantFilter,
    exported_at: DateTime<Utc>,
) -> Result<Export, ApiError> {
    let mut assigner = PathAssigner::default();
    let mut blobs: HashMap<String, i64> = HashMap::new();
    let mut texts: Vec<(String, Vec<u8>)> = Vec::new();

    let models = gather_models(
        db,
        model_ids,
        placement,
        filter,
        &mut assigner,
        &mut blobs,
        &mut texts,
    )
    .await?;
    let bundles = gather_bundles(
        db,
        bundle_ids,
        placement,
        &mut assigner,
        &mut blobs,
        &mut texts,
    )
    .await?;

    // Creators referenced by any exported model or bundle.
    let mut creator_ids: HashSet<Uuid> = HashSet::new();
    creator_ids.extend(models.iter().filter_map(|m| m.creator_id));
    creator_ids.extend(bundles.iter().filter_map(|b| b.creator_id));
    let creator_ids: Vec<Uuid> = creator_ids.into_iter().collect();
    let creators = sqlx::query!(
        r#"SELECT id, name, kind::text as "kind!", url, notes
           FROM creators WHERE id = ANY($1)"#,
        &creator_ids
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| Creator {
        id: r.id,
        name: r.name,
        kind: r.kind,
        url: r.url,
        notes: r.notes,
    })
    .collect();

    // Tag vocabulary actually used (model tags, bundle tags, category tags).
    let mut tag_names: HashSet<String> = HashSet::new();
    for m in &models {
        tag_names.extend(m.tags.iter().cloned());
    }
    for b in &bundles {
        tag_names.extend(b.tags.iter().cloned());
        tag_names.extend(b.categories.iter().map(|c| c.tag.clone()));
    }
    let mut tags: Vec<String> = tag_names.into_iter().collect();
    tags.sort();

    // Variant tag vocabulary used, carrying descriptions.
    let mut vtag_names: HashSet<String> = HashSet::new();
    for m in &models {
        for v in &m.variants {
            vtag_names.extend(v.tags.iter().cloned());
        }
    }
    let vtag_list: Vec<String> = vtag_names.into_iter().collect();
    let variant_tags = sqlx::query!(
        r#"SELECT name::text as "name!", description
           FROM variant_tags WHERE name = ANY($1) ORDER BY name"#,
        &vtag_list
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| VariantTag {
        name: r.name,
        description: r.description,
    })
    .collect();

    let mut blob_list: Vec<Blob> = blobs
        .into_iter()
        .map(|(sha256, size)| Blob { sha256, size })
        .collect();
    blob_list.sort_by(|a, b| a.sha256.cmp(&b.sha256));

    let readme = format!(
        "MeshTrove export ({SCHEMA})\n\
         \n\
         This archive holds {} model(s) and {} bundle(s). Browse the folders to\n\
         see files as they are named and organised in the collection; a bundle's\n\
         member models sit inside it, under its category tabs. manifest.json is\n\
         the authoritative description used to restore this archive into a\n\
         MeshTrove instance.\n",
        models.len(),
        bundles.len(),
    );
    texts.push(("README.txt".into(), readme.into_bytes()));

    Ok(Export {
        manifest: Manifest {
            schema: SCHEMA.into(),
            exported_at,
            creators,
            tags,
            variant_tags,
            models,
            bundles,
            blobs: blob_list,
        },
        texts,
    })
}

// Intermediate rows: each `sqlx::query!` arm produces its own anonymous type, so
// the owner-branch queries map into these shared structs before assembly.
struct RawFile {
    id: Uuid,
    blob_sha256: String,
    path: String,
    filename: String,
    mime: Option<String>,
    kind: String,
    created_at: DateTime<Utc>,
    size: i64,
}

struct RawImage {
    blob_sha256: String,
    kind: String,
    mime: Option<String>,
    width: Option<i32>,
    height: Option<i32>,
    is_primary: bool,
    sort_order: i32,
    renderer: Option<String>,
    renderer_config: Option<serde_json::Value>,
    source_file_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    size: i64,
}

#[allow(clippy::too_many_arguments)]
async fn gather_models(
    db: &PgPool,
    model_ids: &[Uuid],
    placement: &Placement,
    filter: &VariantFilter,
    assigner: &mut PathAssigner,
    blobs: &mut HashMap<String, i64>,
    texts: &mut Vec<(String, Vec<u8>)>,
) -> Result<Vec<Model>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT id, name, slug, creator_id, source_url, license,
                  purchase_price::float8 as "purchase_price?", purchase_date,
                  order_ref, created_at
           FROM models WHERE id = ANY($1) ORDER BY created_at, id"#,
        model_ids
    )
    .fetch_all(db)
    .await?;

    let mut models = Vec::with_capacity(rows.len());
    for r in rows {
        let base = placement
            .model_base
            .get(&r.id)
            .cloned()
            .unwrap_or_else(|| camel(&r.name));

        let tags = sqlx::query_scalar!(
            r#"SELECT t.name::text as "name!" FROM model_tags mt
               JOIN tags t ON t.id = mt.tag_id
               WHERE mt.model_id = $1 ORDER BY t.name"#,
            r.id
        )
        .fetch_all(db)
        .await?;

        let aliases = sqlx::query_scalar!(
            r#"SELECT alias::text as "alias!" FROM model_aliases
               WHERE model_id = $1 ORDER BY alias"#,
            r.id
        )
        .fetch_all(db)
        .await?;

        let descriptions = gather_descriptions_model(db, r.id).await?;
        if let Some(current) = descriptions.last() {
            texts.push((
                format!("{base}/description.md"),
                current.body_md.clone().into_bytes(),
            ));
        }

        let variant_rows = sqlx::query!(
            r#"SELECT id, name, print_notes, derived_from_variant_id, created_at
               FROM model_variants WHERE model_id = $1 ORDER BY created_at, id"#,
            r.id
        )
        .fetch_all(db)
        .await?;

        let mut variants = Vec::with_capacity(variant_rows.len());
        for v in variant_rows {
            let vtags = sqlx::query_scalar!(
                r#"SELECT vt.name::text as "name!" FROM variant_tag_assignments a
                   JOIN variant_tags vt ON vt.id = a.tag_id
                   WHERE a.variant_id = $1 ORDER BY vt.name"#,
                v.id
            )
            .fetch_all(db)
            .await?;
            // Skip variants the export's filter rules out (e.g. "unsupported").
            if !filter.matches(&vtags) {
                continue;
            }
            let vbase = format!("{base}/variants/{}", variant_dir(&vtags));
            let files = build_files(variant_files(db, v.id).await?, &vbase, assigner, blobs);
            let images = build_images(variant_images(db, v.id).await?, &vbase, assigner, blobs);
            variants.push(Variant {
                id: v.id,
                name: v.name,
                print_notes: v.print_notes,
                derived_from_variant_id: v.derived_from_variant_id,
                created_at: v.created_at,
                tags: vtags,
                files,
                images,
            });
        }

        let doc_base = format!("{base}/documents");
        let files = build_files(model_files(db, r.id).await?, &doc_base, assigner, blobs);
        let images = build_images(model_images(db, r.id).await?, &base, assigner, blobs);
        let source_archive = gather_source_archive_model(db, r.id).await?;

        models.push(Model {
            id: r.id,
            name: r.name,
            slug: r.slug,
            creator_id: r.creator_id,
            source_url: r.source_url,
            license: r.license,
            purchase_price: r.purchase_price,
            purchase_date: r.purchase_date,
            order_ref: r.order_ref,
            created_at: r.created_at,
            tags,
            aliases,
            descriptions,
            variants,
            files,
            images,
            source_archive,
        });
    }
    Ok(models)
}

async fn gather_bundles(
    db: &PgPool,
    bundle_ids: &[Uuid],
    placement: &Placement,
    assigner: &mut PathAssigner,
    blobs: &mut HashMap<String, i64>,
    texts: &mut Vec<(String, Vec<u8>)>,
) -> Result<Vec<Bundle>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT id, name, slug, creator_id, source_url, kind::text as "kind!",
                  name_autogenerated, created_at
           FROM bundles WHERE id = ANY($1) ORDER BY created_at, id"#,
        bundle_ids
    )
    .fetch_all(db)
    .await?;

    let mut bundles = Vec::with_capacity(rows.len());
    for r in rows {
        let base = placement
            .bundle_base
            .get(&r.id)
            .cloned()
            .unwrap_or_else(|| camel(&r.name));

        let tags = sqlx::query_scalar!(
            r#"SELECT t.name::text as "name!" FROM bundle_tags bt
               JOIN tags t ON t.id = bt.tag_id
               WHERE bt.bundle_id = $1 ORDER BY t.name"#,
            r.id
        )
        .fetch_all(db)
        .await?;

        let descriptions = gather_descriptions_bundle(db, r.id).await?;
        if let Some(current) = descriptions.last() {
            texts.push((
                format!("{base}/description.md"),
                current.body_md.clone().into_bytes(),
            ));
        }

        let member_ids = sqlx::query_scalar!(
            "SELECT model_id FROM bundle_models WHERE bundle_id = $1",
            r.id
        )
        .fetch_all(db)
        .await?;

        let child_ids = sqlx::query_scalar!(
            "SELECT child_bundle_id FROM bundle_children WHERE parent_bundle_id = $1",
            r.id
        )
        .fetch_all(db)
        .await?;

        let categories = sqlx::query!(
            r#"SELECT t.name::text as "tag!", bc.position FROM bundle_categories bc
               JOIN tags t ON t.id = bc.tag_id
               WHERE bc.bundle_id = $1 ORDER BY bc.position"#,
            r.id
        )
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|c| Category {
            tag: c.tag,
            position: c.position,
        })
        .collect();

        let doc_base = format!("{base}/documents");
        let files = build_files(bundle_files(db, r.id).await?, &doc_base, assigner, blobs);
        let images = build_images(bundle_images(db, r.id).await?, &base, assigner, blobs);
        let source_archive = gather_source_archive_bundle(db, r.id).await?;

        bundles.push(Bundle {
            id: r.id,
            name: r.name,
            slug: r.slug,
            creator_id: r.creator_id,
            source_url: r.source_url,
            kind: r.kind,
            name_autogenerated: r.name_autogenerated,
            created_at: r.created_at,
            tags,
            descriptions,
            member_ids,
            child_ids,
            categories,
            files,
            images,
            source_archive,
        });
    }
    Ok(bundles)
}

fn build_files(
    rows: Vec<RawFile>,
    base: &str,
    assigner: &mut PathAssigner,
    blobs: &mut HashMap<String, i64>,
) -> Vec<File> {
    let mut files = Vec::with_capacity(rows.len());
    for r in rows {
        let mut segments = vec![base.to_string()];
        segments.extend(rel_path(&r.path));
        segments.push(seg(&r.filename));
        let archive_path = assigner.assign(&segments.join("/"), &r.blob_sha256);
        blobs.insert(r.blob_sha256.clone(), r.size);
        files.push(File {
            id: r.id,
            blob_sha256: r.blob_sha256,
            path: r.path,
            filename: r.filename,
            mime: r.mime,
            kind: r.kind,
            created_at: r.created_at,
            archive_path,
        });
    }
    files
}

fn build_images(
    rows: Vec<RawImage>,
    base: &str,
    assigner: &mut PathAssigner,
    blobs: &mut HashMap<String, i64>,
) -> Vec<Image> {
    let mut images = Vec::with_capacity(rows.len());
    for (n, r) in rows.into_iter().enumerate() {
        let ext = ext_for(r.mime.as_deref());
        let name = if r.is_primary {
            format!("primary.{ext}")
        } else {
            format!("{n:02}.{ext}")
        };
        let archive_path = assigner.assign(&format!("{base}/images/{name}"), &r.blob_sha256);
        blobs.insert(r.blob_sha256.clone(), r.size);
        images.push(Image {
            blob_sha256: r.blob_sha256,
            kind: r.kind,
            mime: r.mime,
            width: r.width,
            height: r.height,
            is_primary: r.is_primary,
            sort_order: r.sort_order,
            renderer: r.renderer,
            renderer_config: r.renderer_config,
            source_file_id: r.source_file_id,
            created_at: r.created_at,
            archive_path,
        });
    }
    images
}

async fn model_files(db: &PgPool, id: Uuid) -> Result<Vec<RawFile>, ApiError> {
    Ok(sqlx::query_as!(
        RawFile,
        r#"SELECT f.id, f.blob_sha256, f.path, f.filename, f.mime,
                  f.kind::text as "kind!", f.created_at, b.size
           FROM files f JOIN blobs b ON b.sha256 = f.blob_sha256
           WHERE f.model_id = $1 ORDER BY f.path, f.filename"#,
        id
    )
    .fetch_all(db)
    .await?)
}

async fn variant_files(db: &PgPool, id: Uuid) -> Result<Vec<RawFile>, ApiError> {
    Ok(sqlx::query_as!(
        RawFile,
        r#"SELECT f.id, f.blob_sha256, f.path, f.filename, f.mime,
                  f.kind::text as "kind!", f.created_at, b.size
           FROM files f JOIN blobs b ON b.sha256 = f.blob_sha256
           WHERE f.variant_id = $1 ORDER BY f.path, f.filename"#,
        id
    )
    .fetch_all(db)
    .await?)
}

async fn bundle_files(db: &PgPool, id: Uuid) -> Result<Vec<RawFile>, ApiError> {
    Ok(sqlx::query_as!(
        RawFile,
        r#"SELECT f.id, f.blob_sha256, f.path, f.filename, f.mime,
                  f.kind::text as "kind!", f.created_at, b.size
           FROM files f JOIN blobs b ON b.sha256 = f.blob_sha256
           WHERE f.bundle_id = $1 ORDER BY f.path, f.filename"#,
        id
    )
    .fetch_all(db)
    .await?)
}

async fn model_images(db: &PgPool, id: Uuid) -> Result<Vec<RawImage>, ApiError> {
    Ok(sqlx::query_as!(
        RawImage,
        r#"SELECT i.blob_sha256, i.kind::text as "kind!", i.mime, i.width, i.height,
                  i.is_primary, i.sort_order, i.renderer, i.renderer_config,
                  i.source_file_id, i.created_at, b.size
           FROM images i JOIN blobs b ON b.sha256 = i.blob_sha256
           WHERE i.model_id = $1 ORDER BY i.is_primary DESC, i.sort_order, i.created_at"#,
        id
    )
    .fetch_all(db)
    .await?)
}

async fn variant_images(db: &PgPool, id: Uuid) -> Result<Vec<RawImage>, ApiError> {
    Ok(sqlx::query_as!(
        RawImage,
        r#"SELECT i.blob_sha256, i.kind::text as "kind!", i.mime, i.width, i.height,
                  i.is_primary, i.sort_order, i.renderer, i.renderer_config,
                  i.source_file_id, i.created_at, b.size
           FROM images i JOIN blobs b ON b.sha256 = i.blob_sha256
           WHERE i.variant_id = $1 ORDER BY i.is_primary DESC, i.sort_order, i.created_at"#,
        id
    )
    .fetch_all(db)
    .await?)
}

async fn bundle_images(db: &PgPool, id: Uuid) -> Result<Vec<RawImage>, ApiError> {
    Ok(sqlx::query_as!(
        RawImage,
        r#"SELECT i.blob_sha256, i.kind::text as "kind!", i.mime, i.width, i.height,
                  i.is_primary, i.sort_order, i.renderer, i.renderer_config,
                  i.source_file_id, i.created_at, b.size
           FROM images i JOIN blobs b ON b.sha256 = i.blob_sha256
           WHERE i.bundle_id = $1 ORDER BY i.is_primary DESC, i.sort_order, i.created_at"#,
        id
    )
    .fetch_all(db)
    .await?)
}

async fn gather_descriptions_model(
    db: &PgPool,
    model_id: Uuid,
) -> Result<Vec<Description>, ApiError> {
    Ok(sqlx::query!(
        r#"SELECT body_md, label::text as "label?", created_at
           FROM model_description_revisions
           WHERE model_id = $1 ORDER BY created_at, id"#,
        model_id
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| Description {
        body_md: r.body_md,
        label: r.label,
        created_at: r.created_at,
    })
    .collect())
}

async fn gather_descriptions_bundle(
    db: &PgPool,
    bundle_id: Uuid,
) -> Result<Vec<Description>, ApiError> {
    Ok(sqlx::query!(
        r#"SELECT body_md, label::text as "label?", created_at
           FROM bundle_description_revisions
           WHERE bundle_id = $1 ORDER BY created_at, id"#,
        bundle_id
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| Description {
        body_md: r.body_md,
        label: r.label,
        created_at: r.created_at,
    })
    .collect())
}

async fn gather_source_archive_model(
    db: &PgPool,
    model_id: Uuid,
) -> Result<Option<SourceArchive>, ApiError> {
    Ok(sqlx::query!(
        "SELECT filename, sha256, size, imported_at FROM source_archives
         WHERE model_id = $1 ORDER BY imported_at LIMIT 1",
        model_id
    )
    .fetch_optional(db)
    .await?
    .map(|r| SourceArchive {
        filename: r.filename,
        sha256: r.sha256,
        size: r.size,
        imported_at: r.imported_at,
    }))
}

async fn gather_source_archive_bundle(
    db: &PgPool,
    bundle_id: Uuid,
) -> Result<Option<SourceArchive>, ApiError> {
    Ok(sqlx::query!(
        "SELECT filename, sha256, size, imported_at FROM source_archives
         WHERE bundle_id = $1 ORDER BY imported_at LIMIT 1",
        bundle_id
    )
    .fetch_optional(db)
    .await?
    .map(|r| SourceArchive {
        filename: r.filename,
        sha256: r.sha256,
        size: r.size,
        imported_at: r.imported_at,
    }))
}

// ---------------------------------------------------------------------------
// reading an archive blob back: peek the manifest, stage the blobs.
// ---------------------------------------------------------------------------

/// Every (archive_path, blob_sha256) an export writes, across models, their
/// variants, and bundles.
pub fn blob_entries(manifest: &Manifest) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    for m in &manifest.models {
        for f in &m.files {
            out.push((f.archive_path.as_str(), f.blob_sha256.as_str()));
        }
        for img in &m.images {
            out.push((img.archive_path.as_str(), img.blob_sha256.as_str()));
        }
        for v in &m.variants {
            for f in &v.files {
                out.push((f.archive_path.as_str(), f.blob_sha256.as_str()));
            }
            for img in &v.images {
                out.push((img.archive_path.as_str(), img.blob_sha256.as_str()));
            }
        }
    }
    for b in &manifest.bundles {
        for f in &b.files {
            out.push((f.archive_path.as_str(), f.blob_sha256.as_str()));
        }
        for img in &b.images {
            out.push((img.archive_path.as_str(), img.blob_sha256.as_str()));
        }
    }
    out
}

/// Read just `manifest.json` from a stored zip blob. A zip's directory is at the
/// tail and one named entry is a direct seek, so this never unpacks the rest of
/// a (possibly enormous) archive. Returns `None` when the blob is missing, is not
/// a zip, has no manifest, or the manifest is not a MeshTrove export we speak.
pub async fn read_manifest_from_blob(
    store: &FsBlobStore,
    sha256: &str,
) -> Result<Option<Manifest>, ApiError> {
    let path = store.path_for(sha256);
    tokio::task::spawn_blocking(move || {
        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(ApiError::Internal(e.into())),
        };
        let mut zip = match zip::ZipArchive::new(file) {
            Ok(z) => z,
            Err(_) => return Ok(None),
        };
        let Ok(entry) = zip.by_name("manifest.json") else {
            return Ok(None);
        };
        let Ok(manifest) = serde_json::from_reader::<_, Manifest>(entry) else {
            return Ok(None);
        };
        Ok((manifest.schema == SCHEMA).then_some(manifest))
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
}

/// Stream every blob a manifest references out of the archive blob and into the
/// content-addressed store, verifying each hash. Reads one entry per distinct
/// blob (the readable tree may repeat bytes; the store dedups them anyway).
pub async fn stage_blobs(
    store: &FsBlobStore,
    archive_sha: &str,
    manifest: &Manifest,
    tmp_dir: &FsPath,
) -> Result<(), ApiError> {
    // Each blob's bytes can be read from any file/image entry referencing it.
    let mut sha_to_path: HashMap<&str, &str> = HashMap::new();
    for (archive_path, sha) in blob_entries(manifest) {
        sha_to_path.entry(sha).or_insert(archive_path);
    }
    let plan: Vec<(String, String)> = manifest
        .blobs
        .iter()
        .map(|b| {
            sha_to_path
                .get(b.sha256.as_str())
                .map(|p| (b.sha256.clone(), p.to_string()))
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "manifest blob {} is referenced by no file",
                        b.sha256
                    ))
                })
        })
        .collect::<Result<_, _>>()?;

    let archive_path = store.path_for(archive_sha);
    let tmp_dir = tmp_dir.to_path_buf();
    let temps = tokio::task::spawn_blocking(move || -> Result<Vec<(String, PathBuf)>, ApiError> {
        let file = std::fs::File::open(&archive_path).map_err(|e| ApiError::Internal(e.into()))?;
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|e| ApiError::BadRequest(format!("archive is not a zip: {e}")))?;
        let mut out = Vec::with_capacity(plan.len());
        for (sha, entry_path) in plan {
            let mut entry = zip
                .by_name(&entry_path)
                .map_err(|_| ApiError::BadRequest(format!("archive is missing {entry_path}")))?;
            let tmp = tmp_dir.join(format!("blob-{}", Uuid::new_v4()));
            let mut w = std::fs::File::create(&tmp).map_err(|e| ApiError::Internal(e.into()))?;
            std::io::copy(&mut entry, &mut w).map_err(|e| ApiError::Internal(e.into()))?;
            out.push((sha, tmp));
        }
        Ok(out)
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))??;

    for (expected, tmp) in temps {
        let f = tokio::fs::File::open(&tmp)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        let stream = ReaderStream::new(f).map_err(anyhow::Error::from);
        let stored = store.put(Box::pin(stream)).await?;
        let _ = tokio::fs::remove_file(&tmp).await;
        if stored.sha256 != expected {
            return Err(ApiError::BadRequest(format!(
                "archive blob content does not match its hash ({expected})"
            )));
        }
    }
    Ok(())
}

/// Write an export to `path` as a zip: the manifest, the readable text files,
/// then each blob streamed byte-for-byte out of the store. Stored (uncompressed)
/// so already-packed model data streams straight through, and zip64
/// (`large_file`) keeps a multi-gigabyte member legal.
pub fn build_zip(store: &FsBlobStore, path: &FsPath, export: &Export) -> Result<(), ApiError> {
    use std::io::Write;
    let file = std::fs::File::create(path).map_err(|e| ApiError::Internal(e.into()))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);

    let manifest_json =
        serde_json::to_vec_pretty(&export.manifest).map_err(|e| ApiError::Internal(e.into()))?;
    zip.start_file("manifest.json", opts).map_err(zip_err)?;
    zip.write_all(&manifest_json)
        .map_err(|e| ApiError::Internal(e.into()))?;

    for (name, bytes) in &export.texts {
        zip.start_file(name, opts).map_err(zip_err)?;
        zip.write_all(bytes)
            .map_err(|e| ApiError::Internal(e.into()))?;
    }

    // One entry per distinct archive_path (a path maps to exactly one sha).
    let mut written: HashSet<&str> = HashSet::new();
    for (archive_path, sha) in blob_entries(&export.manifest) {
        if !written.insert(archive_path) {
            continue;
        }
        let blob_path = store.path_for(sha);
        let mut reader = std::fs::File::open(&blob_path).map_err(|e| {
            ApiError::Internal(anyhow::anyhow!("blob {sha} missing from store: {e}"))
        })?;
        zip.start_file(archive_path, opts).map_err(zip_err)?;
        std::io::copy(&mut reader, &mut zip).map_err(|e| ApiError::Internal(e.into()))?;
    }

    zip.finish().map_err(zip_err)?;
    Ok(())
}

fn zip_err(e: zip::result::ZipError) -> ApiError {
    ApiError::Internal(anyhow::anyhow!("zip error: {e}"))
}

// ---------------------------------------------------------------------------
// restore: write a manifest back into an instance.
// ---------------------------------------------------------------------------

/// Which entities to import as a fresh copy even though one with the same slug
/// already exists. Anything present and *not* named here is skipped and its
/// references resolve to the existing row.
#[derive(Default)]
pub struct RestoreOptions {
    pub fresh: HashSet<Uuid>,
}

#[derive(Serialize, Default)]
pub struct RestoreSummary {
    pub models_created: usize,
    pub models_skipped: usize,
    pub bundles_created: usize,
    pub bundles_skipped: usize,
    pub files: usize,
    pub images: usize,
    pub blobs: usize,
}

/// Apply a manifest. The blob *bytes* must already be in the store (the route
/// stages them before calling this); here we only record the `blobs` rows and
/// build the entities. Everything runs in one transaction, so a failure leaves
/// no half-restored model behind — only orphaned (GC-able) blob bytes.
pub async fn restore(
    state: &AppState,
    user: &User,
    manifest: &Manifest,
    options: &RestoreOptions,
) -> Result<RestoreSummary, ApiError> {
    let mut tx = state.db.begin().await?;
    let mut summary = RestoreSummary::default();

    // Blob rows first: files and images FK to them.
    for blob in &manifest.blobs {
        let done = sqlx::query!(
            "INSERT INTO blobs (sha256, size) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            blob.sha256,
            blob.size,
        )
        .execute(&mut *tx)
        .await?;
        summary.blobs += done.rows_affected() as usize;
    }

    // Creators: resolve by name (reuse an existing one), else create. Map the
    // manifest-local id to the resolved id.
    let mut creator_map: HashMap<Uuid, Uuid> = HashMap::new();
    for c in &manifest.creators {
        if let Some(id) = resolve_creator(&mut tx, c).await? {
            creator_map.insert(c.id, id);
        }
    }

    // Tag and variant-tag vocabularies, upserted by name.
    let mut tag_map: HashMap<String, Uuid> = HashMap::new();
    for name in &manifest.tags {
        let tag = upsert_tag(&mut *tx, name).await?;
        tag_map.insert(name.to_lowercase(), tag.id);
    }
    for vt in &manifest.variant_tags {
        upsert_variant_tag(&mut tx, vt).await?;
    }
    let mut vtag_map: HashMap<String, Uuid> = HashMap::new();
    for vt in &manifest.variant_tags {
        let id = sqlx::query_scalar!("SELECT id FROM variant_tags WHERE name = $1", vt.name)
            .fetch_one(&mut *tx)
            .await?;
        vtag_map.insert(vt.name.to_lowercase(), id);
    }

    // Existing slugs, to decide skip vs create.
    let model_slugs: Vec<String> = manifest.models.iter().map(|m| m.slug.clone()).collect();
    let existing_models: HashMap<String, Uuid> = sqlx::query!(
        "SELECT slug, id FROM models WHERE slug = ANY($1)",
        &model_slugs
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| (r.slug, r.id))
    .collect();

    let mut model_map: HashMap<Uuid, Uuid> = HashMap::new();
    for m in &manifest.models {
        if let Some(&existing) = existing_models.get(&m.slug)
            && !options.fresh.contains(&m.id)
        {
            model_map.insert(m.id, existing);
            summary.models_skipped += 1;
            continue;
        }
        let new_id = create_model(
            state,
            &mut tx,
            user,
            m,
            &creator_map,
            &tag_map,
            &vtag_map,
            &mut summary,
        )
        .await?;
        model_map.insert(m.id, new_id);
        summary.models_created += 1;
    }

    // Bundles: create/resolve every bundle first (so child links can resolve),
    // then wire children in a second pass.
    let bundle_slugs: Vec<String> = manifest.bundles.iter().map(|b| b.slug.clone()).collect();
    let existing_bundles: HashMap<String, Uuid> = sqlx::query!(
        "SELECT slug, id FROM bundles WHERE slug = ANY($1)",
        &bundle_slugs
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| (r.slug, r.id))
    .collect();

    let mut bundle_map: HashMap<Uuid, Uuid> = HashMap::new();
    let mut created_bundle_ids: HashSet<Uuid> = HashSet::new();
    for b in &manifest.bundles {
        if let Some(&existing) = existing_bundles.get(&b.slug)
            && !options.fresh.contains(&b.id)
        {
            bundle_map.insert(b.id, existing);
            summary.bundles_skipped += 1;
            continue;
        }
        let new_id = create_bundle(
            state,
            &mut tx,
            user,
            b,
            &creator_map,
            &tag_map,
            &model_map,
            &mut summary,
        )
        .await?;
        bundle_map.insert(b.id, new_id);
        created_bundle_ids.insert(b.id);
        summary.bundles_created += 1;
    }

    // Child links (only for bundles we actually created; a skipped one keeps its
    // own structure untouched).
    for b in &manifest.bundles {
        if !created_bundle_ids.contains(&b.id) {
            continue;
        }
        let parent = bundle_map[&b.id];
        for child in &b.child_ids {
            if let Some(&child_id) = bundle_map.get(child) {
                sqlx::query!(
                    "INSERT INTO bundle_children (parent_bundle_id, child_bundle_id)
                     VALUES ($1, $2) ON CONFLICT DO NOTHING",
                    parent,
                    child_id,
                )
                .execute(&mut *tx)
                .await?;
            }
        }
    }

    tx.commit().await?;
    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
async fn create_model(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    user: &User,
    m: &Model,
    creator_map: &HashMap<Uuid, Uuid>,
    tag_map: &HashMap<String, Uuid>,
    vtag_map: &HashMap<String, Uuid>,
    summary: &mut RestoreSummary,
) -> Result<Uuid, ApiError> {
    let slug = crate::routes::models::unique_slug(state, &m.name, Some(&m.slug), None).await?;
    let creator_id = m.creator_id.and_then(|id| creator_map.get(&id).copied());
    let model_id = sqlx::query_scalar!(
        r#"INSERT INTO models
             (name, slug, creator_id, source_url, license, purchase_price,
              purchase_date, order_ref, created_by, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6::float8::numeric(10,2), $7, $8, $9, $10, $10)
           RETURNING id"#,
        m.name,
        slug,
        creator_id,
        m.source_url,
        m.license,
        m.purchase_price,
        m.purchase_date,
        m.order_ref,
        user.id,
        m.created_at,
    )
    .fetch_one(&mut *tx)
    .await?;

    for d in &m.descriptions {
        sqlx::query!(
            "INSERT INTO model_description_revisions
               (model_id, body_md, label, created_by, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            model_id,
            d.body_md,
            d.label,
            user.id,
            d.created_at,
        )
        .execute(&mut *tx)
        .await?;
    }

    for name in &m.tags {
        if let Some(&tag_id) = tag_map.get(&name.to_lowercase()) {
            sqlx::query!(
                "INSERT INTO model_tags (model_id, tag_id) VALUES ($1, $2)
                 ON CONFLICT DO NOTHING",
                model_id,
                tag_id,
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    for alias in &m.aliases {
        sqlx::query!(
            "INSERT INTO model_aliases (model_id, alias) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            model_id,
            alias,
        )
        .execute(&mut *tx)
        .await?;
    }

    // Variants: create rows first (no derived_from), map ids, then all files,
    // then fix up derived_from, tag assignments and images.
    let mut variant_map: HashMap<Uuid, Uuid> = HashMap::new();
    let mut file_map: HashMap<Uuid, Uuid> = HashMap::new();
    for v in &m.variants {
        let vid = sqlx::query_scalar!(
            "INSERT INTO model_variants (model_id, name, print_notes, created_by, created_at)
             VALUES ($1, $2, $3, $4, $5) RETURNING id",
            model_id,
            v.name,
            v.print_notes,
            user.id,
            v.created_at,
        )
        .fetch_one(&mut *tx)
        .await?;
        variant_map.insert(v.id, vid);
    }

    for v in &m.variants {
        let vid = variant_map[&v.id];
        for f in &v.files {
            let new_fid = insert_file(tx, FileTarget::Variant(vid), f).await?;
            file_map.insert(f.id, new_fid);
            summary.files += 1;
        }
        for name in &v.tags {
            if let Some(&tag_id) = vtag_map.get(&name.to_lowercase()) {
                sqlx::query!(
                    "INSERT INTO variant_tag_assignments (variant_id, tag_id)
                     VALUES ($1, $2) ON CONFLICT DO NOTHING",
                    vid,
                    tag_id,
                )
                .execute(&mut *tx)
                .await?;
            }
        }
    }

    for f in &m.files {
        let new_fid = insert_file(tx, FileTarget::Model(model_id), f).await?;
        file_map.insert(f.id, new_fid);
        summary.files += 1;
    }
    for v in &m.variants {
        if let Some(src) = v.derived_from_variant_id
            && let Some(&mapped) = variant_map.get(&src)
        {
            sqlx::query!(
                "UPDATE model_variants SET derived_from_variant_id = $2 WHERE id = $1",
                variant_map[&v.id],
                mapped,
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    for v in &m.variants {
        let vid = variant_map[&v.id];
        for img in &v.images {
            insert_image(tx, ImageTarget::Variant(vid), img, &file_map, user.id).await?;
            summary.images += 1;
        }
    }
    for img in &m.images {
        insert_image(tx, ImageTarget::Model(model_id), img, &file_map, user.id).await?;
        summary.images += 1;
    }

    if let Some(sa) = &m.source_archive {
        sqlx::query!(
            "INSERT INTO source_archives (model_id, filename, sha256, size, imported_at)
             VALUES ($1, $2, $3, $4, $5)",
            model_id,
            sa.filename,
            sa.sha256,
            sa.size,
            sa.imported_at,
        )
        .execute(&mut *tx)
        .await?;
    }

    Ok(model_id)
}

#[allow(clippy::too_many_arguments)]
async fn create_bundle(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    user: &User,
    b: &Bundle,
    creator_map: &HashMap<Uuid, Uuid>,
    tag_map: &HashMap<String, Uuid>,
    model_map: &HashMap<Uuid, Uuid>,
    summary: &mut RestoreSummary,
) -> Result<Uuid, ApiError> {
    let slug = crate::routes::bundles::unique_slug(state, &b.name, Some(&b.slug), None).await?;
    let creator_id = b.creator_id.and_then(|id| creator_map.get(&id).copied());
    let kind = b.kind.as_str();
    let bundle_id = sqlx::query_scalar!(
        r#"INSERT INTO bundles
             (name, slug, creator_id, source_url, kind, name_autogenerated,
              created_by, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5::bundle_kind, $6, $7, $8, $8)
           RETURNING id"#,
        b.name,
        slug,
        creator_id,
        b.source_url,
        kind as _,
        b.name_autogenerated,
        user.id,
        b.created_at,
    )
    .fetch_one(&mut *tx)
    .await?;

    for d in &b.descriptions {
        sqlx::query!(
            "INSERT INTO bundle_description_revisions
               (bundle_id, body_md, label, created_by, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            bundle_id,
            d.body_md,
            d.label,
            user.id,
            d.created_at,
        )
        .execute(&mut *tx)
        .await?;
    }

    for name in &b.tags {
        if let Some(&tag_id) = tag_map.get(&name.to_lowercase()) {
            sqlx::query!(
                "INSERT INTO bundle_tags (bundle_id, tag_id) VALUES ($1, $2)
                 ON CONFLICT DO NOTHING",
                bundle_id,
                tag_id,
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    for member in &b.member_ids {
        if let Some(&mid) = model_map.get(member) {
            sqlx::query!(
                "INSERT INTO bundle_models (bundle_id, model_id) VALUES ($1, $2)
                 ON CONFLICT DO NOTHING",
                bundle_id,
                mid,
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    for cat in &b.categories {
        if let Some(&tag_id) = tag_map.get(&cat.tag.to_lowercase()) {
            sqlx::query!(
                "INSERT INTO bundle_categories (bundle_id, tag_id, position)
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                bundle_id,
                tag_id,
                cat.position,
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    for f in &b.files {
        insert_file(tx, FileTarget::Bundle(bundle_id), f).await?;
        summary.files += 1;
    }
    let empty_file_map: HashMap<Uuid, Uuid> = HashMap::new();
    for img in &b.images {
        insert_image(
            tx,
            ImageTarget::Bundle(bundle_id),
            img,
            &empty_file_map,
            user.id,
        )
        .await?;
        summary.images += 1;
    }

    if let Some(sa) = &b.source_archive {
        sqlx::query!(
            "INSERT INTO source_archives (bundle_id, filename, sha256, size, imported_at)
             VALUES ($1, $2, $3, $4, $5)",
            bundle_id,
            sa.filename,
            sa.sha256,
            sa.size,
            sa.imported_at,
        )
        .execute(&mut *tx)
        .await?;
    }

    Ok(bundle_id)
}

enum FileTarget {
    Model(Uuid),
    Variant(Uuid),
    Bundle(Uuid),
}

async fn insert_file(
    tx: &mut sqlx::PgConnection,
    target: FileTarget,
    f: &File,
) -> Result<Uuid, ApiError> {
    let (model_id, variant_id, bundle_id) = match target {
        FileTarget::Model(id) => (Some(id), None, None),
        FileTarget::Variant(id) => (None, Some(id), None),
        FileTarget::Bundle(id) => (None, None, Some(id)),
    };
    let kind = f.kind.as_str();
    Ok(sqlx::query_scalar!(
        r#"INSERT INTO files
             (blob_sha256, model_id, variant_id, bundle_id, path, filename, mime, kind, created_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8::file_kind, $9)
           RETURNING id"#,
        f.blob_sha256,
        model_id,
        variant_id,
        bundle_id,
        f.path,
        f.filename,
        f.mime,
        kind as _,
        f.created_at,
    )
    .fetch_one(&mut *tx)
    .await?)
}

enum ImageTarget {
    Model(Uuid),
    Variant(Uuid),
    Bundle(Uuid),
}

async fn insert_image(
    tx: &mut sqlx::PgConnection,
    target: ImageTarget,
    img: &Image,
    file_map: &HashMap<Uuid, Uuid>,
    user_id: Uuid,
) -> Result<(), ApiError> {
    let (model_id, variant_id, bundle_id) = match target {
        ImageTarget::Model(id) => (Some(id), None, None),
        ImageTarget::Variant(id) => (None, Some(id), None),
        ImageTarget::Bundle(id) => (None, None, Some(id)),
    };
    let source_file_id = img.source_file_id.and_then(|id| file_map.get(&id).copied());
    let kind = img.kind.as_str();
    sqlx::query!(
        r#"INSERT INTO images
             (blob_sha256, model_id, variant_id, bundle_id, kind, source_file_id,
              renderer, renderer_config, mime, width, height, is_primary, sort_order,
              created_by, created_at)
           VALUES ($1, $2, $3, $4, $5::image_kind, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)"#,
        img.blob_sha256,
        model_id,
        variant_id,
        bundle_id,
        kind as _,
        source_file_id,
        img.renderer,
        img.renderer_config,
        img.mime,
        img.width,
        img.height,
        img.is_primary,
        img.sort_order,
        user_id,
        img.created_at,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Resolve a manifest creator to a `creators` row: reuse one with the same name
/// (case-insensitively), else create it carrying its kind/url/notes. Mirrors the
/// find-then-insert in `patch.rs` (creators have no unique name index).
async fn resolve_creator(
    tx: &mut sqlx::PgConnection,
    c: &Creator,
) -> Result<Option<Uuid>, ApiError> {
    let name = c.name.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if let Some(id) = sqlx::query_scalar!(
        "SELECT id FROM creators WHERE lower(name) = lower($1) ORDER BY created_at LIMIT 1",
        name,
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        return Ok(Some(id));
    }
    let kind = c.kind.as_str();
    let id = sqlx::query_scalar!(
        "INSERT INTO creators (name, kind, url, notes)
         VALUES ($1, $2::creator_kind, $3, $4) RETURNING id",
        name,
        kind as _,
        c.url,
        c.notes,
    )
    .fetch_one(&mut *tx)
    .await?;
    Ok(Some(id))
}

async fn upsert_variant_tag(tx: &mut sqlx::PgConnection, vt: &VariantTag) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO variant_tags (name, description) VALUES ($1, $2)
         ON CONFLICT (name) DO NOTHING",
        vt.name,
        vt.description,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_joins_words() {
        assert_eq!(camel("Warrior Mummy"), "WarriorMummy");
        assert_eq!(camel("knight-errant"), "KnightErrant");
        assert_eq!(camel("Gold"), "Gold");
        // Internal casing (an acronym) is preserved past the first letter.
        assert_eq!(camel("STL knight"), "STLKnight");
        // Nothing usable collapses to a placeholder, never an empty segment.
        assert_eq!(camel("!!!"), "_");
    }

    #[test]
    fn variant_dir_is_order_insensitive_and_anonymous_is_base() {
        assert_eq!(variant_dir(&[]), "_base");
        assert_eq!(
            variant_dir(&["supported".into(), "32mm".into()]),
            variant_dir(&["32mm".into(), "supported".into()])
        );
        assert_eq!(
            variant_dir(&["32mm".into(), "supported".into()]),
            "32mm-supported"
        );
    }

    #[test]
    fn seg_keeps_safe_chars_and_neutralises_traversal() {
        assert_eq!(seg("Heroes"), "Heroes");
        assert_eq!(seg("a/b"), "a_b");
        assert_eq!(seg(".."), "_");
        assert_eq!(seg("  "), "_");
    }

    #[test]
    fn assigner_reuses_same_bytes_at_same_path_but_suffixes_a_clash() {
        let mut a = PathAssigner::default();
        // Same readable path, same bytes: one archive entry.
        assert_eq!(a.assign("m/knight.stl", "sha1"), "m/knight.stl");
        assert_eq!(a.assign("m/knight.stl", "sha1"), "m/knight.stl");
        // Same readable path, different bytes: the second is suffixed.
        assert_eq!(a.assign("m/knight.stl", "sha2"), "m/knight (2).stl");
        // A different location for shared bytes writes again (readable tree wins).
        assert_eq!(a.assign("n/knight.stl", "sha1"), "n/knight.stl");
    }
}
