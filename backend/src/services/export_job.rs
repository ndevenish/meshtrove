//! export_archive job: build the zip for an `exports` row in the background.
//!
//! A bundle can be gigabytes, so the archive is assembled off the request path.
//! The job reads the export's spec, gathers the manifest, writes the zip under
//! `<store>/exports/<id>.zip` (a one-off artifact, deliberately not a
//! content-addressed blob), and flips the row to `ready` with its size — or to
//! `failed` with the error, so the Exports page can show what went wrong.

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::services::transfer::{self, ExportSpec};
use crate::state::AppState;

#[derive(Deserialize)]
struct ExportPayload {
    export_id: Uuid,
}

/// Directory holding finished export artifacts, beside the blob store.
pub fn export_dir(state: &AppState) -> std::path::PathBuf {
    state.config.store_dir.join("exports")
}

pub async fn export_archive(state: &AppState, payload: &Value) -> Result<()> {
    let payload: ExportPayload =
        serde_json::from_value(payload.clone()).context("bad export_archive payload")?;
    let export_id = payload.export_id;

    let result = build(state, export_id).await;
    if let Err(error) = &result {
        // Record the failure on the row so the user sees it, then let the job
        // machinery mark the job failed too (it will not retry a row that is
        // already gone, and a transient error still gets its normal retries).
        let _ = sqlx::query!(
            "UPDATE exports SET status = 'failed', error = $2, updated_at = now()
             WHERE id = $1 AND status <> 'ready'",
            export_id,
            error.to_string(),
        )
        .execute(&state.db)
        .await;
    }
    result
}

async fn build(state: &AppState, export_id: Uuid) -> Result<()> {
    let row = sqlx::query!("SELECT name, spec FROM exports WHERE id = $1", export_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| anyhow!("export {export_id} no longer exists"))?;

    let spec: ExportSpec =
        serde_json::from_value(row.spec).context("stored export spec is unreadable")?;

    // gather + zip can be slow and CPU/IO-bound; do them without holding a txn.
    let export = transfer::gather_export(&state.db, &spec, Utc::now())
        .await
        .map_err(|e| anyhow!("gathering export: {e}"))?;

    let dir = export_dir(state);
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(format!("{export_id}.zip"));

    let store = state.store.clone();
    let build_path = path.clone();
    tokio::task::spawn_blocking(move || transfer::build_zip(&store, &build_path, &export))
        .await
        .context("export zip task panicked")?
        .map_err(|e| anyhow!("building zip: {e}"))?;

    let size = tokio::fs::metadata(&path).await?.len() as i64;
    let filename = format!("{}.meshtrove.zip", crate::util::slugify(&row.name));
    sqlx::query!(
        "UPDATE exports SET status = 'ready', size = $2, filename = $3, error = NULL,
                            updated_at = now()
         WHERE id = $1",
        export_id,
        size,
        filename,
    )
    .execute(&state.db)
    .await?;

    tracing::info!(export = %export_id, size, "export built");
    Ok(())
}
