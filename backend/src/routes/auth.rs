//! Hand-rolled username/password auth: argon2id hashes, private-cookie session.

use anyhow::{Context, Result, anyhow};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use axum_extra::extract::{PrivateCookieJar, cookie::Cookie};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::extractors::{SESSION_COOKIE, User, UserRole};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
}

#[derive(Deserialize, ToSchema)]
pub struct Credentials {
    username: String,
    password: String,
}

async fn hash_password(password: String) -> Result<String> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| anyhow!("hashing password: {e}"))
    })
    .await
    .context("hash task panicked")?
}

async fn verify_password(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash)
            .map(|parsed| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

fn session_cookie(user_id: Uuid) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, user_id.to_string()))
        .path("/")
        .http_only(true)
        .same_site(axum_extra::extract::cookie::SameSite::Lax)
        .permanent()
        .build()
}

/// Open registration. The first ever user becomes admin; everyone else starts
/// as a viewer until promoted.
async fn register(
    State(state): State<AppState>,
    jar: PrivateCookieJar,
    Json(creds): Json<Credentials>,
) -> Response {
    let username = creds.username.trim().to_string();
    if username.len() < 3 || username.len() > 64 {
        return (StatusCode::BAD_REQUEST, "username must be 3-64 characters").into_response();
    }
    if creds.password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            "password must be at least 8 characters",
        )
            .into_response();
    }

    let hash = match hash_password(creds.password).await {
        Ok(hash) => hash,
        Err(error) => {
            tracing::error!(%error, "password hashing failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Race-safe enough for a personal instance: the unique constraint is the
    // real guard; "first user is admin" is best-effort.
    let result = sqlx::query_as!(
        User,
        r#"INSERT INTO users (username, password_hash, role)
           SELECT $1, $2, CASE WHEN EXISTS (SELECT 1 FROM users WHERE id <> $3)
                          THEN 'viewer'::user_role ELSE 'admin'::user_role END
           RETURNING id, username as "username: String", role as "role: UserRole""#,
        username,
        hash,
        Uuid::nil(), // the synthetic anonymous user doesn't count
    )
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(user) => {
            let jar = jar.add(session_cookie(user.id));
            (jar, Json(user)).into_response()
        }
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            (StatusCode::CONFLICT, "username already taken").into_response()
        }
        Err(error) => {
            tracing::error!(%error, "registration failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn login(
    State(state): State<AppState>,
    jar: PrivateCookieJar,
    Json(creds): Json<Credentials>,
) -> Response {
    let row = sqlx::query!(
        r#"SELECT id, username as "username: String", role as "role: UserRole", password_hash
           FROM users WHERE username = $1"#,
        creds.username.trim(),
    )
    .fetch_optional(&state.db)
    .await;

    let row = match row {
        Ok(row) => row,
        Err(error) => {
            tracing::error!(%error, "login query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Verify against a dummy hash when the user doesn't exist so response
    // timing doesn't reveal which usernames are taken.
    const DUMMY_HASH: &str =
        "$argon2id$v=19$m=19456,t=2,p=1$q0R1dWlkc2FsdA$G/eOm5r6gyOQ0dTLXJDXPBhGJ2/BvithZ71sh1E5uWo";
    let (hash, user) = match row {
        Some(row) => (
            row.password_hash.clone(),
            Some(User {
                id: row.id,
                username: row.username,
                role: row.role,
            }),
        ),
        None => (DUMMY_HASH.to_string(), None),
    };

    if verify_password(creds.password, hash).await
        && let Some(user) = user
    {
        let jar = jar.add(session_cookie(user.id));
        return (jar, Json(user)).into_response();
    }
    (StatusCode::UNAUTHORIZED, "invalid username or password").into_response()
}

async fn logout(jar: PrivateCookieJar) -> Response {
    // The removal cookie must carry the same path as the session cookie,
    // otherwise clients keep the original.
    let jar = jar.remove(Cookie::build((SESSION_COOKIE, "")).path("/"));
    (jar, StatusCode::NO_CONTENT).into_response()
}

/// Startup maintenance: the synthetic anonymous admin (target of created_by
/// FKs in --anonymous mode) and the --create-admin account.
pub async fn ensure_startup_users(state: &AppState) -> Result<()> {
    if state.config.anonymous {
        sqlx::query!(
            r#"INSERT INTO users (id, username, password_hash, role)
               VALUES ($1, 'anonymous', '!login-disabled', 'admin')
               ON CONFLICT (id) DO NOTHING"#,
            Uuid::nil(),
        )
        .execute(&state.db)
        .await
        .context("creating anonymous user")?;
    }

    if let Some((username, password)) = state.config.create_admin.clone() {
        let hash = hash_password(password).await?;
        sqlx::query!(
            r#"INSERT INTO users (username, password_hash, role)
               VALUES ($1, $2, 'admin')
               ON CONFLICT (username)
               DO UPDATE SET password_hash = EXCLUDED.password_hash, role = 'admin'"#,
            username,
            hash,
        )
        .execute(&state.db)
        .await
        .context("creating admin user")?;
        tracing::info!(username, "ensured admin user exists");
    }
    Ok(())
}
