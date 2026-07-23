//! Admin-issued API access tokens.
//!
//! An admin mints a token here; a client presents it as `Authorization: Bearer
//! <token>` and the `User` extractor (see `extractors.rs`) resolves it to the
//! admin who created it. Only the SHA-256 hex of the token is stored — the
//! plaintext is returned once, at creation, and never again.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::{User, UserRole};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/admin/tokens", get(list).post(create))
        .route("/api/admin/tokens/{id}", axum::routing::delete(revoke))
}

/// The SHA-256 hex of a token — what the DB stores and what the extractor looks
/// up. Shared with `extractors.rs` so both sides agree on the encoding.
pub(crate) fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// A fresh 256-bit random token, `mtrv_`-prefixed. The prefix is a courtesy to
/// secret scanners (a leaked token is recognizable) and to humans reading logs.
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!(
        "mtrv_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

/// One token as the admin page lists it — everything but the secret, which no
/// longer exists in a form we could show.
#[derive(Serialize, ToSchema)]
struct TokenSummary {
    id: Uuid,
    name: String,
    /// The role the token grants — capped at its owner's live role when used.
    role: UserRole,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    /// The admin the token acts as.
    created_by_username: String,
}

async fn list(
    State(state): State<AppState>,
    user: User,
) -> Result<Json<Vec<TokenSummary>>, ApiError> {
    user.require_admin()?;
    let rows = sqlx::query!(
        r#"SELECT t.id, t.name, t.role as "role: UserRole",
                  t.created_at, t.last_used_at, t.expires_at,
                  u.username as "created_by_username: String"
           FROM api_tokens t JOIN users u ON u.id = t.created_by
           ORDER BY t.created_at DESC"#,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| TokenSummary {
                id: r.id,
                name: r.name,
                role: r.role,
                created_at: r.created_at,
                last_used_at: r.last_used_at,
                expires_at: r.expires_at,
                created_by_username: r.created_by_username,
            })
            .collect(),
    ))
}

#[derive(Deserialize, ToSchema)]
struct CreateToken {
    name: String,
    /// The role the token grants. Omit for a full-admin token (the default and
    /// prior behaviour); set `editor` or `viewer` for a lesser, safer token.
    #[serde(default)]
    role: Option<UserRole>,
    /// Optional expiry; omit or null for a token that never expires.
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
}

/// The one and only time the plaintext token is returned — the client must copy
/// it now, as only its hash is kept.
#[derive(Serialize, ToSchema)]
struct NewToken {
    id: Uuid,
    name: String,
    role: UserRole,
    token: String,
    created_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<CreateToken>,
) -> Result<Json<NewToken>, ApiError> {
    user.require_admin()?;
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest("a token name is required".into()));
    }

    // Default to admin, matching a token minted before roles existed. Any role
    // is allowed here — only an admin reaches this handler, and the extractor
    // caps the effective role at the owner's live role anyway.
    let role = input.role.unwrap_or(UserRole::Admin);
    let token = generate_token();
    let hash = hash_token(&token);
    let row = sqlx::query!(
        r#"INSERT INTO api_tokens (name, token_hash, role, created_by, expires_at)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, created_at"#,
        name,
        hash,
        role as UserRole,
        user.id,
        input.expires_at,
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(NewToken {
        id: row.id,
        name,
        role,
        token,
        created_at: row.created_at,
        expires_at: input.expires_at,
    }))
}

async fn revoke(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    user.require_admin()?;
    let deleted = sqlx::query!("DELETE FROM api_tokens WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
