//! Shared API error type: handlers return `Result<T, ApiError>` and get
//! consistent status codes without leaking internals.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::extractors::AuthError;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Auth(#[from] AuthError),
    /// The blob store ran out of disk. Its own variant because "internal error"
    /// tells the user nothing they can act on, and this one they can fix.
    #[error("the server has run out of disk space")]
    OutOfSpace,
    #[error(transparent)]
    Internal(anyhow::Error),
}

/// ENOSPC on every unix; `io::ErrorKind::StorageFull` is still unstable.
const ENOSPC: i32 = 28;

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        let out_of_space = error
            .chain()
            .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
            .any(|io| io.raw_os_error() == Some(ENOSPC));
        if out_of_space {
            ApiError::OutOfSpace
        } else {
            ApiError::Internal(error)
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        match error {
            sqlx::Error::RowNotFound => ApiError::NotFound,
            other => ApiError::Internal(other.into()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found").into_response(),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg).into_response(),
            ApiError::Auth(e) => e.into_response(),
            ApiError::OutOfSpace => {
                tracing::error!("blob store is out of disk space");
                (
                    StatusCode::INSUFFICIENT_STORAGE,
                    "the server has run out of disk space",
                )
                    .into_response()
            }
            ApiError::Internal(error) => {
                tracing::error!(%error, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full disk reaches the handler as an io::Error buried under whatever
    /// context the blob store added on the way up — it is only actionable to the
    /// user if we still recognise it down there.
    #[test]
    fn enospc_is_classified_as_out_of_space() {
        let io = std::io::Error::from_raw_os_error(ENOSPC);
        let error = anyhow::Error::from(io).context("writing /store/tmp/abc");
        assert!(matches!(ApiError::from(error), ApiError::OutOfSpace));
    }

    #[test]
    fn other_io_errors_stay_internal() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope");
        let error = anyhow::Error::from(io).context("writing /store/tmp/abc");
        assert!(matches!(ApiError::from(error), ApiError::Internal(_)));
    }
}
