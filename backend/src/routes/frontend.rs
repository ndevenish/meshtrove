//! Catch-all route mounted last: serves the built SPA in production, or
//! transparently proxies everything (including Vite's HMR WebSocket) to the
//! Vite dev server when running with --dev.

use std::path::{Component, PathBuf};

use axum::{
    body::Body,
    extract::{
        FromRequestParts, State, WebSocketUpgrade,
        ws::{Message as AxumMessage, WebSocket},
    },
    http::{Request, StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::protocol::Message as TMessage;

use crate::state::AppState;

/// Path prefixes owned by the backend; a miss here is a real 404, never SPA HTML.
const BACKEND_PREFIXES: &[&str] = &["/api/", "/auth/", "/docs/", "/openapi.json"];

pub async fn frontend_handler(State(state): State<AppState>, req: Request<Body>) -> Response {
    let path = req.uri().path().to_string();
    if BACKEND_PREFIXES
        .iter()
        .any(|p| path.starts_with(p) || path == p.trim_end_matches('/'))
    {
        return StatusCode::NOT_FOUND.into_response();
    }

    let is_websocket = req
        .headers()
        .get(header::UPGRADE)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"));

    if state.config.dev_mode {
        if is_websocket {
            let (mut parts, _body) = req.into_parts();
            return match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
                Ok(ws) => proxy_websocket(ws, &state, &parts.uri),
                Err(rejection) => rejection.into_response(),
            };
        }
        proxy_to_vite(&state, req.uri()).await
    } else {
        if is_websocket {
            // WebSocket proxying is a dev-mode facility only.
            return StatusCode::BAD_REQUEST.into_response();
        }
        serve_static(&state, &path).await
    }
}

/// Production: serve the built asset if it exists; fall back to index.html for
/// extension-less paths so client-side routes survive deep links / refresh.
async fn serve_static(state: &AppState, path: &str) -> Response {
    let rel = path.trim_start_matches('/');
    let rel_path = PathBuf::from(rel);
    if rel_path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return StatusCode::NOT_FOUND.into_response();
    }

    let mut target = state.config.static_dir.join(&rel_path);
    let has_extension = rel_path.extension().is_some();
    if rel.is_empty() || !has_extension {
        if !tokio::fs::try_exists(&target).await.unwrap_or(false) || rel.is_empty() {
            target = state.config.static_dir.join("index.html");
        }
    }

    match tokio::fs::read(&target).await {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&target).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.to_string())], bytes).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Development: forward the request to Vite, copying status and content-type,
/// so the browser gets on-the-fly compilation through the backend's origin.
async fn proxy_to_vite(state: &AppState, uri: &Uri) -> Response {
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    // Url's Display always renders a trailing "/" path; trim it or paths like
    // /@vite/client become //@vite/client, which Vite serves as index.html.
    let target = format!(
        "{}{}",
        state.config.vite_url.as_str().trim_end_matches('/'),
        path_and_query
    );

    match reqwest::get(&target).await {
        Ok(upstream) => {
            let status =
                StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let mut response = Response::builder().status(status);
            if let Some(ct) = upstream.headers().get(header::CONTENT_TYPE) {
                response = response.header(header::CONTENT_TYPE, ct.as_bytes());
            }
            response
                .body(Body::from_stream(upstream.bytes_stream()))
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
        }
        Err(_) => Html(
            "<h1>Vite dev server is not running</h1>\
             <p>Start it with: <code>cd frontend && npm run dev</code></p>",
        )
        .into_response(),
    }
}

/// Proxy a WebSocket upgrade (Vite HMR) through to the dev server.
fn proxy_websocket(ws: WebSocketUpgrade, state: &AppState, uri: &Uri) -> Response {
    let vite = &state.config.vite_url;
    let target = format!(
        "ws://{}:{}{}",
        vite.host_str().unwrap_or("localhost"),
        vite.port().unwrap_or(5173),
        uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/")
    );
    ws.on_upgrade(move |client| async move {
        match tokio_tungstenite::connect_async(&target).await {
            Ok((upstream, _)) => bridge_websockets(client, upstream).await,
            Err(error) => tracing::warn!(%target, %error, "failed to reach Vite HMR websocket"),
        }
    })
}

async fn bridge_websockets<S>(client: WebSocket, upstream: tokio_tungstenite::WebSocketStream<S>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut client_tx, mut client_rx) = client.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();

    let to_upstream = async {
        while let Some(Ok(msg)) = client_rx.next().await {
            let msg = match msg {
                AxumMessage::Text(t) => TMessage::text(t.as_str()),
                AxumMessage::Binary(b) => TMessage::Binary(b),
                AxumMessage::Ping(p) => TMessage::Ping(p),
                AxumMessage::Pong(p) => TMessage::Pong(p),
                AxumMessage::Close(_) => break,
            };
            if upstream_tx.send(msg).await.is_err() {
                break;
            }
        }
    };
    let to_client = async {
        while let Some(Ok(msg)) = upstream_rx.next().await {
            let msg = match msg {
                TMessage::Text(t) => AxumMessage::Text(t.as_str().into()),
                TMessage::Binary(b) => AxumMessage::Binary(b),
                TMessage::Ping(p) => AxumMessage::Ping(p),
                TMessage::Pong(p) => AxumMessage::Pong(p),
                TMessage::Close(_) => break,
                TMessage::Frame(_) => continue,
            };
            if client_tx.send(msg).await.is_err() {
                break;
            }
        }
    };

    tokio::select! {
        _ = to_upstream => {}
        _ = to_client => {}
    }
}
