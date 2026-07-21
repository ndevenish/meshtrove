//! Admin-defined custom fields: the extra metadata a model or bundle can carry
//! beyond the fixed schema (a "Printed?" checkbox, a "Material" choice, a star
//! rating, a reference PDF).
//!
//! The vocabulary is a meshtrove-wide setting, not a per-entity one: an admin
//! defines a field once and it becomes available on every model and/or bundle.
//! This module owns the *definitions*; the values live on the model and bundle
//! detail endpoints.

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::StatusCode,
    routing::{get, post},
};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::services::blobstore::BlobStore;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/custom-fields", get(list).post(create))
        .route(
            "/api/custom-fields/{id}",
            axum::routing::put(update).delete(remove),
        )
        // Values: scalars ride along with the owner's edit, but a file-kind one
        // is bytes, so it gets its own multipart endpoint per owner.
        .route(
            "/api/models/{id}/custom-fields/{field_id}/file",
            post(upload_model_file),
        )
        .route(
            "/api/bundles/{id}/custom-fields/{field_id}/file",
            post(upload_bundle_file),
        )
        .route(
            "/api/models/{id}/custom-fields/{field_id}",
            axum::routing::delete(clear_model_value),
        )
        .route(
            "/api/bundles/{id}/custom-fields/{field_id}",
            axum::routing::delete(clear_bundle_value),
        )
        // The store streams to disk; a reference document has no business being
        // capped at axum's default 2MB either.
        .layer(DefaultBodyLimit::disable())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "custom_field_kind", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum CustomFieldKind {
    Text,
    Checkbox,
    /// A fixed enum; the permitted values live in `options.choices`.
    Choice,
    /// Stars, 1..=`options.max`.
    Rating,
    /// A single dropped file, shown on the page but deliberately kept out of the
    /// owner's normal file list.
    File,
}

/// Who may see a field at all — both its value and the fact it exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "custom_field_visibility", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum CustomFieldVisibility {
    /// Everyone, signed in or not.
    Anonymous,
    /// Any real login.
    Viewer,
    Editor,
    Admin,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CustomField {
    pub id: Uuid,
    /// Stable slug; what scraped metadata keys are matched against.
    pub key: String,
    pub name: String,
    pub kind: CustomFieldKind,
    #[schema(value_type = Object)]
    pub options: serde_json::Value,
    pub applies_to_models: bool,
    pub applies_to_bundles: bool,
    /// Writing this field on a bundle copies the value down to member models.
    pub bundle_persists_to_model: bool,
    /// ...even if the member model already had a value of its own.
    pub bundle_persist_overwrites: bool,
    pub visibility: CustomFieldVisibility,
    pub position: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CustomFieldInput {
    pub key: String,
    pub name: String,
    pub kind: CustomFieldKind,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub options: serde_json::Value,
    #[serde(default)]
    pub applies_to_models: bool,
    #[serde(default)]
    pub applies_to_bundles: bool,
    #[serde(default)]
    pub bundle_persists_to_model: bool,
    #[serde(default)]
    pub bundle_persist_overwrites: bool,
    #[serde(default = "default_visibility")]
    pub visibility: CustomFieldVisibility,
    #[serde(default)]
    pub position: i32,
}

fn default_visibility() -> CustomFieldVisibility {
    CustomFieldVisibility::Anonymous
}

/// The default star count when a rating field doesn't name one.
const DEFAULT_RATING_MAX: i64 = 5;
/// More than this and a star row stops being a rating and starts being a slider.
const MAX_RATING_MAX: i64 = 10;

/// Keys are matched against metadata coming from a scrape, so they have to be
/// stable and unambiguous: no spaces, no punctuation beyond `-` and `_`. Case is
/// preserved but not significant (the column is `citext`).
fn validate_key(key: &str) -> Result<String, ApiError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(ApiError::BadRequest("key is required".into()));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::BadRequest(
            "key may only contain letters, digits, '-' and '_'".into(),
        ));
    }
    Ok(key.to_string())
}

/// Reduce the submitted `options` to the canonical shape for the kind, rejecting
/// anything the kind can't use. Storing only what the kind means keeps every
/// reader (renderer, validator, patch ingest) from having to re-guess.
fn normalize_options(
    kind: CustomFieldKind,
    options: &serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    match kind {
        CustomFieldKind::Choice => {
            let choices = options
                .get("choices")
                .and_then(|c| c.as_array())
                .ok_or_else(|| {
                    ApiError::BadRequest("a choice field needs options.choices".into())
                })?;
            let mut cleaned: Vec<String> = Vec::new();
            for choice in choices {
                let text = choice
                    .as_str()
                    .ok_or_else(|| ApiError::BadRequest("choices must be strings".into()))?
                    .trim();
                if text.is_empty() {
                    return Err(ApiError::BadRequest("choices must not be empty".into()));
                }
                if cleaned.iter().any(|existing| existing == text) {
                    return Err(ApiError::BadRequest(format!("duplicate choice {text:?}")));
                }
                cleaned.push(text.to_string());
            }
            if cleaned.is_empty() {
                return Err(ApiError::BadRequest(
                    "a choice field needs at least one choice".into(),
                ));
            }
            Ok(json!({ "choices": cleaned }))
        }
        CustomFieldKind::Rating => {
            let max = match options.get("max") {
                None | Some(serde_json::Value::Null) => DEFAULT_RATING_MAX,
                Some(value) => value
                    .as_i64()
                    .ok_or_else(|| ApiError::BadRequest("options.max must be a number".into()))?,
            };
            if !(1..=MAX_RATING_MAX).contains(&max) {
                return Err(ApiError::BadRequest(format!(
                    "options.max must be between 1 and {MAX_RATING_MAX}"
                )));
            }
            Ok(json!({ "max": max }))
        }
        // Nothing to configure: drop whatever came in rather than storing a
        // setting no reader will ever honour.
        CustomFieldKind::Text | CustomFieldKind::Checkbox | CustomFieldKind::File => Ok(json!({})),
    }
}

/// Everything a definition has to satisfy regardless of how it arrived.
fn validate(input: &CustomFieldInput) -> Result<(String, String, serde_json::Value), ApiError> {
    let key = validate_key(&input.key)?;
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    if !input.applies_to_models && !input.applies_to_bundles {
        return Err(ApiError::BadRequest(
            "a field must apply to models, bundles, or both".into(),
        ));
    }
    // Persistence is a bundle→model flow: it means nothing unless the field
    // lives at both ends.
    if input.bundle_persists_to_model && !(input.applies_to_models && input.applies_to_bundles) {
        return Err(ApiError::BadRequest(
            "persisting to models needs a field that applies to both models and bundles".into(),
        ));
    }
    let options = normalize_options(input.kind, &input.options)?;
    Ok((key, name, options))
}

fn key_conflict(e: sqlx::Error) -> ApiError {
    match e {
        sqlx::Error::Database(ref db) if db.is_unique_violation() => {
            ApiError::Conflict("a custom field with that key already exists".into())
        }
        e => e.into(),
    }
}

/// Every definition, in display order. Editor-gated rather than public: the set
/// of fields includes ones only an admin may see, and the names alone leak.
/// Read-side consumers get their (visibility-filtered) fields embedded in the
/// model/bundle detail responses instead.
async fn list(
    State(state): State<AppState>,
    user: User,
) -> Result<Json<Vec<CustomField>>, ApiError> {
    user.require_editor()?;
    let rows = sqlx::query_as!(
        CustomField,
        r#"SELECT id, key as "key: String", name,
                  kind as "kind: CustomFieldKind", options,
                  applies_to_models, applies_to_bundles,
                  bundle_persists_to_model, bundle_persist_overwrites,
                  visibility as "visibility: CustomFieldVisibility", position
           FROM custom_fields
           ORDER BY position, name"#,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<CustomFieldInput>,
) -> Result<Json<CustomField>, ApiError> {
    user.require_admin()?;
    let (key, name, options) = validate(&input)?;
    let row = sqlx::query_as!(
        CustomField,
        r#"INSERT INTO custom_fields
             (key, name, kind, options, applies_to_models, applies_to_bundles,
              bundle_persists_to_model, bundle_persist_overwrites, visibility, position)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id, key as "key: String", name,
                     kind as "kind: CustomFieldKind", options,
                     applies_to_models, applies_to_bundles,
                     bundle_persists_to_model, bundle_persist_overwrites,
                     visibility as "visibility: CustomFieldVisibility", position"#,
        key,
        name,
        input.kind as CustomFieldKind,
        options,
        input.applies_to_models,
        input.applies_to_bundles,
        input.bundle_persists_to_model,
        input.bundle_persist_overwrites,
        input.visibility as CustomFieldVisibility,
        input.position,
    )
    .fetch_one(&state.db)
    .await
    .map_err(key_conflict)?;
    Ok(Json(row))
}

/// Changing a field's `kind` is allowed but does not rewrite the values already
/// stored under the old kind — they stay as they are and are re-validated the
/// next time someone writes them.
async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<CustomFieldInput>,
) -> Result<Json<CustomField>, ApiError> {
    user.require_admin()?;
    let (key, name, options) = validate(&input)?;
    let row = sqlx::query_as!(
        CustomField,
        r#"UPDATE custom_fields SET
             key = $2, name = $3, kind = $4, options = $5,
             applies_to_models = $6, applies_to_bundles = $7,
             bundle_persists_to_model = $8, bundle_persist_overwrites = $9,
             visibility = $10, position = $11, updated_at = now()
           WHERE id = $1
           RETURNING id, key as "key: String", name,
                     kind as "kind: CustomFieldKind", options,
                     applies_to_models, applies_to_bundles,
                     bundle_persists_to_model, bundle_persist_overwrites,
                     visibility as "visibility: CustomFieldVisibility", position"#,
        id,
        key,
        name,
        input.kind as CustomFieldKind,
        options,
        input.applies_to_models,
        input.applies_to_bundles,
        input.bundle_persists_to_model,
        input.bundle_persist_overwrites,
        input.visibility as CustomFieldVisibility,
        input.position,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(key_conflict)?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(row))
}

/// Dropping a definition drops every value stored under it (ON DELETE CASCADE),
/// including any file-kind blobs' `files` rows — the blobs themselves are freed
/// by the next GC sweep like any other unreferenced bytes.
async fn remove(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    user.require_admin()?;
    sqlx::query!("DELETE FROM custom_fields WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// values
// ---------------------------------------------------------------------------

/// Which side of the model/bundle divide a value hangs off. A value has exactly
/// one owner, the same way a file does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueOwner {
    Model(Uuid),
    Bundle(Uuid),
}

impl ValueOwner {
    /// The pair of nullable owner columns, in `(model_id, bundle_id)` order.
    fn ids(self) -> (Option<Uuid>, Option<Uuid>) {
        match self {
            ValueOwner::Model(id) => (Some(id), None),
            ValueOwner::Bundle(id) => (None, Some(id)),
        }
    }

    async fn created_by(self, state: &AppState) -> Result<Uuid, ApiError> {
        match self {
            ValueOwner::Model(id) => crate::routes::models::model_created_by(state, id).await,
            ValueOwner::Bundle(id) => crate::routes::bundles::bundle_created_by(state, id).await,
        }
    }
}

/// The file behind a file-kind value. Downloaded through the ordinary
/// `/api/files/{id}/download`, which gates it on the field's visibility.
#[derive(Debug, Serialize, ToSchema)]
pub struct CustomFieldFile {
    pub file_id: Uuid,
    pub filename: String,
    pub mime: Option<String>,
    pub size: i64,
}

/// One field as it appears on a model or bundle: the definition, plus whatever
/// this owner has stored under it. Every *applicable and visible* field is
/// listed, set or not, so an editor sees the blanks it could fill in.
#[derive(Debug, Serialize, ToSchema)]
pub struct CustomFieldValueDetail {
    pub field: CustomField,
    /// Unset is null. Always null for a file-kind field — see `file`.
    #[schema(value_type = Object)]
    pub value: Option<serde_json::Value>,
    pub file: Option<CustomFieldFile>,
}

/// One scalar write, as carried in a model/bundle edit. A null `value` clears.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CustomFieldValueInput {
    pub field_id: Uuid,
    #[schema(value_type = Object)]
    pub value: Option<serde_json::Value>,
}

/// Every field applicable to this owner that `user` is allowed to see, in
/// display order, with its value. Embedded in the model and bundle detail
/// responses.
pub async fn fetch_values(
    state: &AppState,
    owner: ValueOwner,
    user: &User,
) -> Result<Vec<CustomFieldValueDetail>, ApiError> {
    let (model_id, bundle_id) = owner.ids();
    // `IS NOT DISTINCT FROM` on both owner columns picks exactly this owner's
    // value with one query for either side: the null column matches the null
    // stored in the row that isn't the owner.
    let rows = sqlx::query!(
        // Every `cf.` column is annotated non-null: they are NOT NULL on the
        // preserved side of the LEFT JOINs, but sqlx reads nullability off the
        // query plan, which moves with the table statistics — leave them bare
        // and the build passes or fails depending on what is in the database.
        r#"SELECT cf.id as "id!", cf.key as "key!: String", cf.name as "name!",
                  cf.kind as "kind!: CustomFieldKind", cf.options as "options!",
                  cf.applies_to_models as "applies_to_models!",
                  cf.applies_to_bundles as "applies_to_bundles!",
                  cf.bundle_persists_to_model as "bundle_persists_to_model!",
                  cf.bundle_persist_overwrites as "bundle_persist_overwrites!",
                  cf.visibility as "visibility!: CustomFieldVisibility",
                  cf.position as "position!",
                  v.value,
                  f.id as "file_id?", f.filename as "filename?", f.mime as "mime?",
                  b.size as "size?"
           FROM custom_fields cf
           LEFT JOIN custom_field_values v ON v.field_id = cf.id
                AND v.model_id IS NOT DISTINCT FROM $1
                AND v.bundle_id IS NOT DISTINCT FROM $2
           LEFT JOIN files f ON f.custom_field_value_id = v.id
           LEFT JOIN blobs b ON b.sha256 = f.blob_sha256
           WHERE CASE WHEN $1::uuid IS NULL THEN cf.applies_to_bundles ELSE cf.applies_to_models END
           ORDER BY cf.position, cf.name"#,
        model_id,
        bundle_id,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(rows
        .into_iter()
        .filter(|r| user.can_see(r.visibility))
        .map(|r| CustomFieldValueDetail {
            file: match (r.file_id, r.filename, r.size) {
                (Some(file_id), Some(filename), Some(size)) => Some(CustomFieldFile {
                    file_id,
                    filename,
                    mime: r.mime,
                    size,
                }),
                _ => None,
            },
            value: r.value,
            field: CustomField {
                id: r.id,
                key: r.key,
                name: r.name,
                kind: r.kind,
                options: r.options,
                applies_to_models: r.applies_to_models,
                applies_to_bundles: r.applies_to_bundles,
                bundle_persists_to_model: r.bundle_persists_to_model,
                bundle_persist_overwrites: r.bundle_persist_overwrites,
                visibility: r.visibility,
                position: r.position,
            },
        })
        .collect())
}

/// Coerce a submitted value into what the kind stores, or `None` for "clear
/// it". A blank text box and an unticked rating are erasures, not values: the
/// UI has no separate "unset" gesture, so the empty form state has to mean it.
pub(crate) fn validate_value(
    field: &CustomField,
    value: &serde_json::Value,
) -> Result<Option<serde_json::Value>, ApiError> {
    let name = &field.name;
    match field.kind {
        CustomFieldKind::Text => {
            let text = value
                .as_str()
                .ok_or_else(|| ApiError::BadRequest(format!("{name} takes text")))?
                .trim();
            Ok((!text.is_empty()).then(|| json!(text)))
        }
        CustomFieldKind::Checkbox => {
            let ticked = value
                .as_bool()
                .ok_or_else(|| ApiError::BadRequest(format!("{name} takes true or false")))?;
            Ok(Some(json!(ticked)))
        }
        CustomFieldKind::Choice => {
            let choice = value
                .as_str()
                .ok_or_else(|| ApiError::BadRequest(format!("{name} takes one of its choices")))?
                .trim();
            if choice.is_empty() {
                return Ok(None);
            }
            let allowed = field
                .options
                .get("choices")
                .and_then(|c| c.as_array())
                .map(|c| c.iter().any(|v| v.as_str() == Some(choice)))
                .unwrap_or(false);
            if !allowed {
                return Err(ApiError::BadRequest(format!(
                    "{choice:?} is not one of {name}'s choices"
                )));
            }
            Ok(Some(json!(choice)))
        }
        CustomFieldKind::Rating => {
            let stars = value
                .as_i64()
                .ok_or_else(|| ApiError::BadRequest(format!("{name} takes a number of stars")))?;
            if stars <= 0 {
                return Ok(None);
            }
            let max = field
                .options
                .get("max")
                .and_then(|m| m.as_i64())
                .unwrap_or(DEFAULT_RATING_MAX);
            if stars > max {
                return Err(ApiError::BadRequest(format!("{name} goes up to {max}")));
            }
            Ok(Some(json!(stars)))
        }
        // A file arrives as bytes on its own endpoint; there is nothing for a
        // JSON edit to say about it.
        CustomFieldKind::File => Err(ApiError::BadRequest(format!(
            "{name} is a file field — upload to it instead"
        ))),
    }
}

/// Load one definition, insisting it exists and applies to this kind of owner.
async fn field_for(
    db: &mut sqlx::PgConnection,
    owner: ValueOwner,
    field_id: Uuid,
) -> Result<CustomField, ApiError> {
    let field = sqlx::query_as!(
        CustomField,
        r#"SELECT id, key as "key: String", name,
                  kind as "kind: CustomFieldKind", options,
                  applies_to_models, applies_to_bundles,
                  bundle_persists_to_model, bundle_persist_overwrites,
                  visibility as "visibility: CustomFieldVisibility", position
           FROM custom_fields WHERE id = $1"#,
        field_id,
    )
    .fetch_optional(&mut *db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let applies = match owner {
        ValueOwner::Model(_) => field.applies_to_models,
        ValueOwner::Bundle(_) => field.applies_to_bundles,
    };
    if !applies {
        return Err(ApiError::BadRequest(format!(
            "{} doesn't apply to {}",
            field.name,
            match owner {
                ValueOwner::Model(_) => "models",
                ValueOwner::Bundle(_) => "bundles",
            }
        )));
    }
    Ok(field)
}

/// Store one already-validated value (or clear it when `value` is None).
/// Returns the value row's id when it still exists.
pub(crate) async fn write_value(
    db: &mut sqlx::PgConnection,
    owner: ValueOwner,
    field_id: Uuid,
    value: Option<&serde_json::Value>,
    editor: Option<Uuid>,
) -> Result<Option<Uuid>, ApiError> {
    let (model_id, bundle_id) = owner.ids();
    let Some(value) = value else {
        // Clearing takes the row with it, and the row takes any file-kind blob's
        // `files` row with it in turn (ON DELETE CASCADE).
        sqlx::query!(
            "DELETE FROM custom_field_values
             WHERE field_id = $1
               AND model_id IS NOT DISTINCT FROM $2
               AND bundle_id IS NOT DISTINCT FROM $3",
            field_id,
            model_id,
            bundle_id,
        )
        .execute(&mut *db)
        .await?;
        return Ok(None);
    };
    let id = match owner {
        ValueOwner::Model(model_id) => {
            sqlx::query_scalar!(
                "INSERT INTO custom_field_values (field_id, model_id, value, updated_by)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (field_id, model_id) WHERE model_id IS NOT NULL
                 DO UPDATE SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by,
                               updated_at = now()
                 RETURNING id",
                field_id,
                model_id,
                value,
                editor,
            )
            .fetch_one(&mut *db)
            .await?
        }
        ValueOwner::Bundle(bundle_id) => {
            sqlx::query_scalar!(
                "INSERT INTO custom_field_values (field_id, bundle_id, value, updated_by)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (field_id, bundle_id) WHERE bundle_id IS NOT NULL
                 DO UPDATE SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by,
                               updated_at = now()
                 RETURNING id",
                field_id,
                bundle_id,
                value,
                editor,
            )
            .fetch_one(&mut *db)
            .await?
        }
    };
    Ok(Some(id))
}

/// The `updated_by` to record: the synthetic `--anonymous` admin has the nil id
/// and no `users` row to point at.
pub(crate) fn editor_id(user: &User) -> Option<Uuid> {
    (!user.id.is_nil()).then_some(user.id)
}

/// Write the scalar values carried by a model/bundle edit, inside that edit's
/// transaction. File-kind fields are rejected here — they have their own
/// endpoint, because their payload is bytes, not JSON.
pub async fn apply_values(
    tx: &mut sqlx::PgConnection,
    owner: ValueOwner,
    inputs: &[CustomFieldValueInput],
    user: &User,
) -> Result<(), ApiError> {
    for input in inputs {
        let field = field_for(tx, owner, input.field_id).await?;
        let value = match &input.value {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => validate_value(&field, value)?,
        };
        write_value(tx, owner, field.id, value.as_ref(), editor_id(user)).await?;
    }
    Ok(())
}

/// The whole vocabulary, keyed by lowercased `key`, for matching scraped
/// metadata against.
pub async fn fields_by_key(
    db: &mut sqlx::PgConnection,
) -> Result<std::collections::HashMap<String, CustomField>, ApiError> {
    Ok(all_fields(db)
        .await?
        .into_iter()
        .map(|f| (f.key.to_lowercase(), f))
        .collect())
}

/// Every definition, in display order.
pub async fn all_fields(db: &mut sqlx::PgConnection) -> Result<Vec<CustomField>, ApiError> {
    let fields = sqlx::query_as!(
        CustomField,
        r#"SELECT id, key as "key: String", name,
                  kind as "kind: CustomFieldKind", options,
                  applies_to_models, applies_to_bundles,
                  bundle_persists_to_model, bundle_persist_overwrites,
                  visibility as "visibility: CustomFieldVisibility", position
           FROM custom_fields ORDER BY position, name"#,
    )
    .fetch_all(&mut *db)
    .await?;
    Ok(fields)
}

/// What one scraped `key: value` pair turns out to be.
#[derive(Debug, PartialEq)]
pub enum Resolved {
    /// Applicable and well-formed; `value` is None when the pair clears the field.
    Value {
        field_id: Uuid,
        value: Option<serde_json::Value>,
    },
    /// Not applicable. A scrape carries whatever the shop page had, so this is
    /// never an error — it is something to *tell* the user about in the preview
    /// and then skip.
    Warning(String),
}

/// Match one scraped pair against the vocabulary. Deliberately total: every
/// input resolves to either a value to write or a warning to show.
pub fn resolve_patch_value(
    fields: &std::collections::HashMap<String, CustomField>,
    owner: ValueOwner,
    key: &str,
    value: &serde_json::Value,
) -> Resolved {
    let Some(field) = fields.get(&key.trim().to_lowercase()) else {
        return Resolved::Warning(format!("no custom field is defined with the key {key:?}"));
    };
    let applies = match owner {
        ValueOwner::Model(_) => field.applies_to_models,
        ValueOwner::Bundle(_) => field.applies_to_bundles,
    };
    if !applies {
        return Resolved::Warning(format!(
            "{} doesn't apply to {}",
            field.name,
            match owner {
                ValueOwner::Model(_) => "models",
                ValueOwner::Bundle(_) => "bundles",
            }
        ));
    }
    // A patch is JSON; a file field's payload is bytes it cannot carry.
    if matches!(field.kind, CustomFieldKind::File) {
        return Resolved::Warning(format!(
            "{} is a file field — a patch can't fill it",
            field.name
        ));
    }
    match validate_value(field, value) {
        Ok(value) => Resolved::Value {
            field_id: field.id,
            value,
        },
        Err(e) => Resolved::Warning(e.to_string()),
    }
}

/// Write the pairs that resolve, collecting a warning for each that doesn't.
pub async fn apply_patch_values(
    db: &mut sqlx::PgConnection,
    fields: &std::collections::HashMap<String, CustomField>,
    owner: ValueOwner,
    values: &std::collections::HashMap<String, serde_json::Value>,
    user: &User,
) -> Result<Vec<String>, ApiError> {
    let mut warnings = Vec::new();
    // Sorted so a patch with several bad keys warns in the same order twice —
    // a HashMap's iteration order would shuffle the preview between runs.
    let mut keys: Vec<&String> = values.keys().collect();
    keys.sort();
    for key in keys {
        match resolve_patch_value(fields, owner, key, &values[key]) {
            Resolved::Value { field_id, value } => {
                write_value(&mut *db, owner, field_id, value.as_ref(), editor_id(user)).await?;
            }
            Resolved::Warning(reason) => warnings.push(reason),
        }
    }
    Ok(warnings)
}

/// Copy a bundle's persisting fields down onto its member models.
///
/// Runs when the bundle's value is *written* — an edit, a patch, a file upload
/// — and only then: a model that joins the bundle later is not backfilled, and
/// clearing the bundle's value doesn't reach in and clear the members' (they
/// were given a value, and taking it back away is not what "persists to model"
/// promises). A member that already has an answer of its own keeps it unless
/// the field is marked to overwrite.
pub async fn persist_bundle_fields(
    db: &mut sqlx::PgConnection,
    bundle_id: Uuid,
    user: &User,
) -> Result<(), ApiError> {
    let fields = sqlx::query!(
        r#"SELECT cf.id as field_id, cf.kind as "kind: CustomFieldKind",
                  cf.bundle_persist_overwrites, v.id as value_id, v.value
           FROM custom_fields cf
           JOIN custom_field_values v ON v.field_id = cf.id AND v.bundle_id = $1
           WHERE cf.bundle_persists_to_model AND cf.applies_to_models"#,
        bundle_id,
    )
    .fetch_all(&mut *db)
    .await?;
    if fields.is_empty() {
        return Ok(());
    }

    let members = sqlx::query_scalar!(
        "SELECT model_id FROM bundle_models WHERE bundle_id = $1",
        bundle_id,
    )
    .fetch_all(&mut *db)
    .await?;
    let editor = editor_id(user);

    for field in &fields {
        for &model_id in &members {
            let occupied = sqlx::query_scalar!(
                "SELECT id FROM custom_field_values WHERE field_id = $1 AND model_id = $2",
                field.field_id,
                model_id,
            )
            .fetch_optional(&mut *db)
            .await?
            .is_some();
            if occupied && !field.bundle_persist_overwrites {
                continue;
            }
            // A file-kind value carries no JSON; its row exists to own the file.
            let value = field.value.clone().unwrap_or(serde_json::Value::Null);
            let target = write_value(
                &mut *db,
                ValueOwner::Model(model_id),
                field.field_id,
                Some(&value),
                editor,
            )
            .await?
            .expect("a written value has a row");
            if matches!(field.kind, CustomFieldKind::File) {
                // Blobs are content-addressed, so "copying" the file is a row
                // insert pointing at bytes that are already on disk.
                sqlx::query!("DELETE FROM files WHERE custom_field_value_id = $1", target)
                    .execute(&mut *db)
                    .await?;
                sqlx::query!(
                    "INSERT INTO files (blob_sha256, custom_field_value_id, path, filename, mime, kind)
                     SELECT f.blob_sha256, $2, f.path, f.filename, f.mime, f.kind
                     FROM files f WHERE f.custom_field_value_id = $1",
                    field.value_id,
                    target,
                )
                .execute(&mut *db)
                .await?;
            }
        }
    }
    Ok(())
}

async fn upload_model_file(
    State(state): State<AppState>,
    user: User,
    Path((id, field_id)): Path<(Uuid, Uuid)>,
    multipart: Multipart,
) -> Result<Json<CustomFieldValueDetail>, ApiError> {
    upload_file(state, user, ValueOwner::Model(id), field_id, multipart).await
}

async fn upload_bundle_file(
    State(state): State<AppState>,
    user: User,
    Path((id, field_id)): Path<(Uuid, Uuid)>,
    multipart: Multipart,
) -> Result<Json<CustomFieldValueDetail>, ApiError> {
    upload_file(state, user, ValueOwner::Bundle(id), field_id, multipart).await
}

/// Replace a file-kind value's file. One `file` part; anything else is ignored.
/// Deliberately *not* routed through the ordinary upload path: a zip attached as
/// a reference document is a reference document, and must not be unpacked into
/// the model the way a dropped archive would be.
async fn upload_file(
    state: AppState,
    user: User,
    owner: ValueOwner,
    field_id: Uuid,
    mut multipart: Multipart,
) -> Result<Json<CustomFieldValueDetail>, ApiError> {
    user.require_can_edit(owner.created_by(&state).await?)?;
    let mut conn = state.db.acquire().await?;
    let field = field_for(&mut conn, owner, field_id).await?;
    if !matches!(field.kind, CustomFieldKind::File) {
        return Err(ApiError::BadRequest(format!(
            "{} is not a file field",
            field.name
        )));
    }

    // As in routes/files.rs: read the body out before answering, or the browser
    // sees a reset connection instead of the error it should have shown.
    let (filename, mime, sha256, size) = match consume_one_file(&state, &mut multipart).await {
        Ok(stored) => stored,
        Err(e) => {
            while let Ok(Some(_)) = multipart.next_field().await {}
            return Err(e);
        }
    };

    // Swap the file over only once the new bytes are safely stored, so a failed
    // upload leaves the previous file in place. A file-kind value carries no
    // JSON of its own; the row exists purely to own the `files` row.
    let value_id = write_value(
        &mut conn,
        owner,
        field.id,
        Some(&json!(null)),
        editor_id(&user),
    )
    .await?
    .expect("a written value has a row");
    sqlx::query!(
        "DELETE FROM files WHERE custom_field_value_id = $1",
        value_id
    )
    .execute(&mut *conn)
    .await?;
    crate::routes::files::insert_file(
        &state,
        crate::routes::files::Owner::CustomFieldValue(value_id),
        &sha256,
        size,
        "",
        &filename,
        mime,
        crate::routes::files::guess_kind(&filename),
    )
    .await?;
    if let ValueOwner::Bundle(bundle_id) = owner {
        persist_bundle_fields(&mut conn, bundle_id, &user).await?;
    }

    one_value(&state, owner, field.id, &user).await.map(Json)
}

/// Pull the first `file` part out of the body and into the blob store.
async fn consume_one_file(
    state: &AppState,
    multipart: &mut Multipart,
) -> Result<(String, Option<String>, String, i64), ApiError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("bad multipart body: {e}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let filename = field
            .file_name()
            .ok_or_else(|| ApiError::BadRequest("file field needs a filename".into()))?
            .to_string();
        let mime = mime_guess::from_path(&filename)
            .first()
            .map(|m| m.to_string());
        let stream = field.map_err(|e| anyhow::Error::new(e).context("upload stream failed"));
        let blob = state.store.put(stream).await?;
        return Ok((filename, mime, blob.sha256, blob.size));
    }
    Err(ApiError::BadRequest("no file field in upload".into()))
}

async fn clear_model_value(
    State(state): State<AppState>,
    user: User,
    Path((id, field_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    clear_value(state, user, ValueOwner::Model(id), field_id).await
}

async fn clear_bundle_value(
    State(state): State<AppState>,
    user: User,
    Path((id, field_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    clear_value(state, user, ValueOwner::Bundle(id), field_id).await
}

/// Unset one field on one owner. Any file-kind blob's `files` row goes with the
/// value row; the bytes themselves are freed by the next GC sweep.
async fn clear_value(
    state: AppState,
    user: User,
    owner: ValueOwner,
    field_id: Uuid,
) -> Result<StatusCode, ApiError> {
    user.require_can_edit(owner.created_by(&state).await?)?;
    let mut conn = state.db.acquire().await?;
    write_value(&mut conn, owner, field_id, None, editor_id(&user)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The one field's entry, as the detail endpoints would render it.
async fn one_value(
    state: &AppState,
    owner: ValueOwner,
    field_id: Uuid,
    user: &User,
) -> Result<CustomFieldValueDetail, ApiError> {
    fetch_values(state, owner, user)
        .await?
        .into_iter()
        .find(|v| v.field.id == field_id)
        .ok_or(ApiError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kind: CustomFieldKind, options: serde_json::Value) -> CustomFieldInput {
        CustomFieldInput {
            key: "printed".into(),
            name: "Printed?".into(),
            kind,
            options,
            applies_to_models: true,
            applies_to_bundles: false,
            bundle_persists_to_model: false,
            bundle_persist_overwrites: false,
            visibility: CustomFieldVisibility::Anonymous,
            position: 0,
        }
    }

    #[test]
    fn keys_are_slugs() {
        assert_eq!(validate_key(" print_status ").unwrap(), "print_status");
        assert_eq!(validate_key("Scale-1").unwrap(), "Scale-1");
        assert!(validate_key("").is_err());
        assert!(validate_key("has space").is_err());
        assert!(validate_key("dots.not.allowed").is_err());
    }

    /// A choice field is its list of choices: blank, duplicate and non-string
    /// entries are rejected rather than silently stored.
    #[test]
    fn choice_options_are_cleaned() {
        let ok = normalize_options(
            CustomFieldKind::Choice,
            &json!({"choices": [" PLA ", "Resin"]}),
        )
        .unwrap();
        assert_eq!(ok, json!({"choices": ["PLA", "Resin"]}));
        for bad in [
            json!({}),
            json!({"choices": []}),
            json!({"choices": ["PLA", "PLA"]}),
            json!({"choices": [""]}),
            json!({"choices": [3]}),
        ] {
            assert!(
                normalize_options(CustomFieldKind::Choice, &bad).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn rating_max_defaults_and_is_bounded() {
        assert_eq!(
            normalize_options(CustomFieldKind::Rating, &json!({})).unwrap(),
            json!({"max": DEFAULT_RATING_MAX})
        );
        assert_eq!(
            normalize_options(CustomFieldKind::Rating, &json!({"max": 3})).unwrap(),
            json!({"max": 3})
        );
        assert!(normalize_options(CustomFieldKind::Rating, &json!({"max": 0})).is_err());
        assert!(normalize_options(CustomFieldKind::Rating, &json!({"max": 11})).is_err());
        assert!(normalize_options(CustomFieldKind::Rating, &json!({"max": "five"})).is_err());
    }

    /// Kinds with nothing to configure store nothing, whatever was sent.
    #[test]
    fn plain_kinds_drop_options() {
        for kind in [
            CustomFieldKind::Text,
            CustomFieldKind::Checkbox,
            CustomFieldKind::File,
        ] {
            assert_eq!(
                normalize_options(kind, &json!({"choices": ["a"], "max": 3})).unwrap(),
                json!({})
            );
        }
    }

    fn field(kind: CustomFieldKind, options: serde_json::Value) -> CustomField {
        CustomField {
            id: Uuid::nil(),
            key: "f".into(),
            name: "Field".into(),
            kind,
            options,
            applies_to_models: true,
            applies_to_bundles: false,
            bundle_persists_to_model: false,
            bundle_persist_overwrites: false,
            visibility: CustomFieldVisibility::Anonymous,
            position: 0,
        }
    }

    /// The edit form has no separate "unset" gesture, so an emptied control has
    /// to mean erase — but an unticked checkbox is a real answer, not a blank.
    #[test]
    fn emptied_controls_clear_the_value() {
        let text = field(CustomFieldKind::Text, json!({}));
        assert_eq!(validate_value(&text, &json!("  ")).unwrap(), None);
        assert_eq!(
            validate_value(&text, &json!(" hi ")).unwrap(),
            Some(json!("hi"))
        );

        let rating = field(CustomFieldKind::Rating, json!({"max": 5}));
        assert_eq!(validate_value(&rating, &json!(0)).unwrap(), None);
        assert_eq!(validate_value(&rating, &json!(3)).unwrap(), Some(json!(3)));

        let checkbox = field(CustomFieldKind::Checkbox, json!({}));
        assert_eq!(
            validate_value(&checkbox, &json!(false)).unwrap(),
            Some(json!(false))
        );
    }

    #[test]
    fn values_are_checked_against_the_kind() {
        let choice = field(
            CustomFieldKind::Choice,
            json!({"choices": ["PLA", "Resin"]}),
        );
        assert_eq!(
            validate_value(&choice, &json!("Resin")).unwrap(),
            Some(json!("Resin"))
        );
        assert_eq!(validate_value(&choice, &json!("")).unwrap(), None);
        assert!(validate_value(&choice, &json!("Steel")).is_err());

        let rating = field(CustomFieldKind::Rating, json!({"max": 3}));
        assert!(validate_value(&rating, &json!(4)).is_err());
        assert!(validate_value(&rating, &json!("three")).is_err());

        assert!(
            validate_value(&field(CustomFieldKind::Checkbox, json!({})), &json!("yes")).is_err()
        );
    }

    /// A file arrives as bytes on its own endpoint; a JSON edit has nothing to
    /// say about it, and quietly ignoring the attempt would hide a real mistake.
    #[test]
    fn a_file_field_cannot_be_written_as_json() {
        let file = field(CustomFieldKind::File, json!({}));
        assert!(validate_value(&file, &json!("something.pdf")).is_err());
    }

    /// A scrape carries whatever the shop page had: a key nobody defined, a
    /// field that lives on the other side of the model/bundle divide, or a value
    /// the kind can't take is something to *report*, never something that fails
    /// the whole patch.
    #[test]
    fn unusable_patch_keys_warn_rather_than_fail() {
        let mut printed = field(CustomFieldKind::Checkbox, json!({}));
        printed.name = "Printed?".into();
        let mut manual = field(CustomFieldKind::File, json!({}));
        manual.applies_to_bundles = true;
        let fields = std::collections::HashMap::from([
            ("printed".to_string(), printed),
            ("manual".to_string(), manual),
        ]);
        let model = ValueOwner::Model(Uuid::nil());
        let bundle = ValueOwner::Bundle(Uuid::nil());

        assert!(matches!(
            resolve_patch_value(&fields, model, "PRINTED", &json!(true)),
            Resolved::Value { .. }
        ));
        for (owner, key, value) in [
            (model, "nonesuch", json!(true)),
            // `printed` is a models-only field in this fixture.
            (bundle, "printed", json!(true)),
            // A patch is JSON; it has no bytes to give a file field.
            (model, "manual", json!("manual.pdf")),
            (model, "printed", json!("yes please")),
        ] {
            assert!(
                matches!(
                    resolve_patch_value(&fields, owner, key, &value),
                    Resolved::Warning(_)
                ),
                "{key} {value}"
            );
        }
    }

    #[test]
    fn a_field_must_apply_somewhere() {
        let mut field = input(CustomFieldKind::Text, json!({}));
        field.applies_to_models = false;
        assert!(validate(&field).is_err());
    }

    /// Bundle→model persistence is a flow between two ends: a field that only
    /// exists at one end can't have it.
    #[test]
    fn persistence_needs_both_ends() {
        let mut field = input(CustomFieldKind::Text, json!({}));
        field.bundle_persists_to_model = true;
        assert!(validate(&field).is_err());
        field.applies_to_bundles = true;
        assert!(validate(&field).is_ok());
    }
}
