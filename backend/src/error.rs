//! Shared API error type: handlers return `Result<T, ApiError>` and get
//! consistent status codes without leaking internals.

use axum::extract::multipart::MultipartError;
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
    /// The client's upload stopped part-way — it hung up mid-stream, or a flaky
    /// connection dropped the body. Not our fault: a 400 the browser can surface,
    /// logged as a warning rather than paging like an internal error would.
    #[error("the upload was interrupted before it finished")]
    UploadInterrupted,
    #[error(transparent)]
    Internal(anyhow::Error),
}

/// ENOSPC on every unix; `io::ErrorKind::StorageFull` is still unstable.
const ENOSPC: i32 = 28;

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        // A failure streaming the multipart body up is the request's problem, not
        // the server's — a MultipartError only ever comes from parsing the
        // client's body, never from any server-internal path. So its presence in
        // the chain means the upload broke on the way in: a truncated body multer
        // calls IncompleteStream, a dropped connection it calls StreamReadFailed
        // (which axum's own status() even maps to a 500). Either way it is the
        // upload that failed, not us — don't log it as an internal error.
        if error.chain().any(|cause| cause.is::<MultipartError>()) {
            return ApiError::UploadInterrupted;
        }
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
            // Client errors were silent, which made a handler-originated 404 (a
            // secondary `fetch_one` coming back empty → sqlx RowNotFound → NotFound)
            // indistinguishable from an unmatched route. Log them so a 4xx from
            // inside a handler always leaves a trace; a truly unmatched route is
            // logged separately by the API fallback in main.rs.
            ApiError::BadRequest(msg) => {
                tracing::warn!(status = 400, %msg, "bad request");
                (StatusCode::BAD_REQUEST, msg).into_response()
            }
            ApiError::NotFound => {
                tracing::warn!(status = 404, "not found (handler)");
                (StatusCode::NOT_FOUND, "not found").into_response()
            }
            ApiError::Conflict(msg) => {
                tracing::warn!(status = 409, %msg, "conflict");
                (StatusCode::CONFLICT, msg).into_response()
            }
            ApiError::Auth(e) => e.into_response(),
            ApiError::OutOfSpace => {
                tracing::error!("blob store is out of disk space");
                (
                    StatusCode::INSUFFICIENT_STORAGE,
                    "the server has run out of disk space",
                )
                    .into_response()
            }
            ApiError::UploadInterrupted => {
                tracing::warn!(
                    status = 400,
                    "upload interrupted (client hung up mid-stream)"
                );
                (
                    StatusCode::BAD_REQUEST,
                    "the upload was interrupted before it finished",
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

    /// A body that breaks mid-upload reaches the handler as a MultipartError
    /// buried under the blob store's context. It is the client hanging up, not a
    /// server fault, so it must classify as UploadInterrupted (a 400) rather than
    /// an internal 500 — which is what made a dropped connection page as an error.
    #[tokio::test]
    async fn interrupted_multipart_is_not_internal() {
        use axum::extract::{FromRequest, Multipart};

        // A real (truncated) multipart: a field opens but the body ends with no
        // closing boundary, exactly as a client hanging up mid-upload leaves it —
        // multer answers that with an incomplete-stream error (a 400).
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"file\"; \
                    filename=\"a.stl\"\r\n\r\npartial data with no closing boundary";
        let request = axum::http::Request::builder()
            .header("content-type", "multipart/form-data; boundary=boundary")
            .body(axum::body::Body::from(body))
            .unwrap();
        let mut multipart = Multipart::from_request(request, &()).await.unwrap();
        let field = multipart.next_field().await.unwrap().unwrap();
        let err = field
            .bytes()
            .await
            .expect_err("a body with no closing boundary must fail to parse");
        assert!(err.status().is_client_error());

        let wrapped = anyhow::Error::new(err).context("upload stream failed");
        assert!(matches!(
            ApiError::from(wrapped),
            ApiError::UploadInterrupted
        ));
    }
}
