//! render_preview job: shell out to an external renderer (f3d by default) to
//! produce a PNG preview of a model file. The renderer command is an
//! admin-global setting; every rendered image records the renderer + config
//! that produced it so stale ones can be found and re-rendered later.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::services::blobstore::BlobStore;
use crate::state::AppState;

pub const RENDERER_SETTING: &str = "renderer";

/// `{input}` and `{output}` placeholders are substituted into args.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct RendererConfig {
    pub tool: String,
    pub args: Vec<String>,
}

impl Default for RendererConfig {
    fn default() -> Self {
        RendererConfig {
            tool: "f3d".to_string(),
            args: vec![
                "{input}".to_string(),
                "--output={output}".to_string(),
                // --no-config: ignore any user config so results are
                // deterministic (no grid/axis/filename overlays)
                "--no-config".to_string(),
                "--resolution=1024,1024".to_string(),
                "--ambient-occlusion".to_string(),
                "--anti-aliasing".to_string(),
                "--camera-direction=-1,-0.6,-1".to_string(),
            ],
        }
    }
}

pub async fn current_config(state: &AppState) -> Result<RendererConfig> {
    let value = sqlx::query_scalar!(
        "SELECT value FROM settings WHERE key = $1",
        RENDERER_SETTING,
    )
    .fetch_optional(&state.db)
    .await?;
    Ok(match value {
        Some(value) => serde_json::from_value(value).context("invalid renderer setting")?,
        None => RendererConfig::default(),
    })
}

#[derive(Deserialize)]
struct RenderPayload {
    /// files.id of the model file to render
    file_id: Uuid,
    /// "add" keeps existing images; "replace" removes the image in
    /// `replace_image_id` after a successful render
    #[serde(default)]
    mode: RenderMode,
    replace_image_id: Option<Uuid>,
}

#[derive(Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
enum RenderMode {
    #[default]
    Add,
    Replace,
}

pub async fn render_preview(state: &AppState, payload: &Value) -> Result<()> {
    let payload: RenderPayload =
        serde_json::from_value(payload.clone()).context("bad render_preview payload")?;
    let config = current_config(state).await?;

    let file = sqlx::query!(
        "SELECT blob_sha256, filename, model_id, variant_id, bundle_id FROM files WHERE id = $1",
        payload.file_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| anyhow!("file {} no longer exists", payload.file_id))?;

    let blob_path = state.store.path_for(&file.blob_sha256);

    // The renderer needs a recognizable extension; the store path has none,
    // so hard-link (fall back to copy) into a temp name preserving it.
    let work_dir = std::env::temp_dir().join(format!("meshtrove-render-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&work_dir).await?;
    let input = work_dir.join(&file.filename);
    if tokio::fs::hard_link(&blob_path, &input).await.is_err() {
        tokio::fs::copy(&blob_path, &input)
            .await
            .context("staging input file")?;
    }
    let output = work_dir.join("preview.png");

    let args: Vec<String> = config
        .args
        .iter()
        .map(|arg| {
            arg.replace("{input}", &input.to_string_lossy())
                .replace("{output}", &output.to_string_lossy())
        })
        .collect();

    let result = tokio::process::Command::new(&config.tool)
        .args(&args)
        .output()
        .await
        .with_context(|| format!("launching renderer {:?}", config.tool));

    let render_outcome = async {
        let output_info = result?;
        if !output_info.status.success() {
            return Err(anyhow!(
                "renderer exited with {}: {}",
                output_info.status,
                String::from_utf8_lossy(&output_info.stderr)
                    .chars()
                    .take(2000)
                    .collect::<String>()
            ));
        }
        if !tokio::fs::try_exists(&output).await? {
            return Err(anyhow!("renderer succeeded but produced no output file"));
        }
        Ok(())
    }
    .await;

    if let Err(error) = render_outcome {
        let _ = tokio::fs::remove_dir_all(&work_dir).await;
        return Err(error);
    }

    // Store the PNG and record the image with renderer provenance.
    let png = tokio::fs::File::open(&output).await?;
    use futures::TryStreamExt;
    let stream = tokio_util::io::ReaderStream::new(png).map_err(anyhow::Error::from);
    let blob = state.store.put(stream).await?;
    let _ = tokio::fs::remove_dir_all(&work_dir).await;

    let config_json = serde_json::to_value(&config)?;
    let mut tx = state.db.begin().await?;
    sqlx::query!(
        "INSERT INTO blobs (sha256, size) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        blob.sha256,
        blob.size,
    )
    .execute(&mut *tx)
    .await?;
    // Delete the image being replaced FIRST so the primary slot it may hold
    // falls through to the new render below.
    if payload.mode == RenderMode::Replace {
        if let Some(old) = payload.replace_image_id {
            sqlx::query!(
                "DELETE FROM images WHERE id = $1 AND kind = 'rendered'",
                old
            )
            .execute(&mut *tx)
            .await?;
        }
    }
    sqlx::query!(
        r#"INSERT INTO images (blob_sha256, model_id, variant_id, bundle_id, kind, mime,
                               source_file_id, renderer, renderer_config, is_primary, created_by)
           SELECT $1, $2, $3, $4, 'rendered', 'image/png', $5, $6, $7,
                  NOT EXISTS (SELECT 1 FROM images i WHERE i.is_primary AND (
                      (i.model_id = $2 AND $2 IS NOT NULL) OR
                      (i.variant_id = $3 AND $3 IS NOT NULL) OR
                      (i.bundle_id = $4 AND $4 IS NOT NULL))),
                  NULL"#,
        blob.sha256,
        file.model_id,
        file.variant_id,
        file.bundle_id,
        payload.file_id,
        config.tool,
        config_json,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    tracing::info!(file = %payload.file_id, tool = %config.tool, "preview rendered");
    Ok(())
}
