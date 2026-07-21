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
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/custom-fields", get(list).post(create))
        .route(
            "/api/custom-fields/{id}",
            axum::routing::put(update).delete(remove),
        )
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
