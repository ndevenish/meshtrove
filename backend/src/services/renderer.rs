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

/// Per-file orientation, layered on top of the global config at render time.
/// Stored on `files.render_overrides` (see migration 0040): which way is up is a
/// property of the mesh, not of the renderer, and the files in a library
/// disagree with each other.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct RenderOverrides {
    /// f3d up axis: `+X`, `-X`, `+Y`, `-Y`, `+Z`, `-Z`. `None` leaves whatever
    /// the global config says (f3d's own default is `+Y`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<String>,
    /// Camera azimuth about the up axis, in degrees — spinning the turntable
    /// the model stands on. Normalised to 0..360; 0 is "no rotation" and is
    /// stored as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turntable: Option<i32>,
}

pub const UP_AXES: [&str; 6] = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];

impl RenderOverrides {
    pub fn is_empty(&self) -> bool {
        self.up.is_none() && self.turntable.is_none()
    }

    /// Reject an axis f3d would not understand, and fold the turntable angle
    /// into 0..360 so `-45` and `315` are the same stored state (and so the UI
    /// can step past either end without accumulating a silly number).
    pub fn normalise(mut self) -> Result<Self> {
        if let Some(up) = &self.up
            && !UP_AXES.contains(&up.as_str())
        {
            return Err(anyhow!("up must be one of {}", UP_AXES.join(", ")));
        }
        self.turntable = match self.turntable {
            Some(degrees) => match degrees.rem_euclid(360) {
                0 => None,
                d => Some(d),
            },
            None => None,
        };
        Ok(self)
    }
}

/// The command line for one render: the global args with the file's own
/// orientation appended.
///
/// Appended, not merged: f3d takes the *last* occurrence of a repeated option
/// (verified against f3d 3.5.0 — `--up=+Y --up=+Z` renders identically to
/// `--up=+Z`), so a config that already sets `--up` is overridden rather than
/// argued with, and one that doesn't simply gains the flag.
fn args_for(config: &RendererConfig, overrides: Option<&RenderOverrides>) -> Vec<String> {
    let mut args = config.args.clone();
    let Some(overrides) = overrides else {
        return args;
    };
    if let Some(up) = &overrides.up {
        args.push(format!("--up={up}"));
    }
    if let Some(degrees) = overrides.turntable {
        args.push(format!("--camera-azimuth-angle={degrees}"));
    }
    args
}

/// The orientation stored against a file, if it has one.
pub async fn overrides_for_file(
    state: &AppState,
    file_id: Uuid,
) -> Result<Option<RenderOverrides>> {
    let value = sqlx::query_scalar!("SELECT render_overrides FROM files WHERE id = $1", file_id,)
        .fetch_optional(&state.db)
        .await?
        .flatten();
    Ok(match value {
        Some(value) => serde_json::from_value(value).context("invalid render_overrides")?,
        None => None,
    })
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

/// Render a stored blob to a PNG via the external renderer, into a fresh temp
/// dir. On success returns `(work_dir, output_png)`; the caller owns `work_dir`
/// and MUST remove it once it has consumed the PNG. On failure the temp dir is
/// cleaned up here. Shared by the `render_preview` job (which persists the PNG as
/// an image) and the on-demand preview endpoint (which streams it and throws it
/// away).
pub async fn render_blob_to_png(
    config: &RendererConfig,
    overrides: Option<&RenderOverrides>,
    blob_path: &std::path::Path,
    filename: &str,
) -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    // The renderer needs a recognizable extension; the store path has none,
    // so hard-link (fall back to copy) into a temp name preserving it.
    let work_dir = std::env::temp_dir().join(format!("meshtrove-render-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&work_dir).await?;
    let input = work_dir.join(filename);
    if tokio::fs::hard_link(blob_path, &input).await.is_err() {
        tokio::fs::copy(blob_path, &input)
            .await
            .context("staging input file")?;
    }
    let output = work_dir.join("preview.png");

    let args: Vec<String> = args_for(config, overrides)
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

    Ok((work_dir, output))
}

pub async fn render_preview(state: &AppState, payload: &Value) -> Result<()> {
    let payload: RenderPayload =
        serde_json::from_value(payload.clone()).context("bad render_preview payload")?;
    let config = current_config(state).await?;

    let file = sqlx::query!(
        "SELECT blob_sha256, filename, model_id, variant_id, bundle_id, render_overrides
         FROM files WHERE id = $1",
        payload.file_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| anyhow!("file {} no longer exists", payload.file_id))?;

    // Read at render time, not from the payload: the file is the one place the
    // orientation lives, so a bulk re-render queued before the fix still picks
    // the fix up.
    let overrides: Option<RenderOverrides> = match &file.render_overrides {
        Some(value) => serde_json::from_value(value.clone()).context("invalid render_overrides")?,
        None => None,
    };

    let blob_path = state.store.path_for(&file.blob_sha256);
    let (work_dir, output) =
        render_blob_to_png(&config, overrides.as_ref(), &blob_path, &file.filename).await?;

    // Store the PNG and record the image with renderer provenance.
    let png = tokio::fs::File::open(&output).await?;
    use futures::TryStreamExt;
    let stream = tokio_util::io::ReaderStream::new(png).map_err(anyhow::Error::from);
    let blob = state.store.put(stream).await?;
    let _ = tokio::fs::remove_dir_all(&work_dir).await;

    let config_json = serde_json::to_value(&config)?;
    let overrides_json = overrides.as_ref().map(serde_json::to_value).transpose()?;
    let mut tx = state.db.begin().await?;
    sqlx::query!(
        "INSERT INTO blobs (sha256, size) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        blob.sha256,
        blob.size,
    )
    .execute(&mut *tx)
    .await?;
    // Replacing an earlier render rewrites that row rather than swapping it for
    // a new one. The id is what the page is holding — the selected thumbnail,
    // the open lightbox, the orientation popover that just queued this job — and
    // the row carries state the picture should not lose on its way past: whether
    // it is the owner's primary, and where it sits in the gallery.
    //
    // Dimensions go back to NULL rather than being kept: they described the old
    // bytes, and the global config that sizes a render can have moved since.
    let replaced = if payload.mode == RenderMode::Replace
        && let Some(old) = payload.replace_image_id
    {
        sqlx::query!(
            "UPDATE images SET blob_sha256 = $1, mime = 'image/png', source_file_id = $2,
                               renderer = $3, renderer_config = $4, render_overrides = $5,
                               width = NULL, height = NULL
             WHERE id = $6 AND kind = 'rendered'",
            blob.sha256,
            payload.file_id,
            config.tool,
            config_json,
            overrides_json,
            old,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0
    } else {
        false
    };

    // Nothing to replace (an "add", or a row deleted while the job queued): the
    // render lands as a new image on whatever owns the file.
    if !replaced {
        sqlx::query!(
            r#"INSERT INTO images (blob_sha256, model_id, variant_id, bundle_id, kind, mime,
                                   source_file_id, renderer, renderer_config, render_overrides,
                                   is_primary, created_by)
               SELECT $1, $2, $3, $4, 'rendered', 'image/png', $5, $6, $7, $8,
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
            overrides_json,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    tracing::info!(file = %payload.file_id, tool = %config.tool, "preview rendered");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overrides(up: Option<&str>, turntable: Option<i32>) -> RenderOverrides {
        RenderOverrides {
            up: up.map(str::to_string),
            turntable,
        }
    }

    #[test]
    fn no_overrides_leaves_the_command_alone() {
        let config = RendererConfig::default();
        assert_eq!(args_for(&config, None), config.args);
        assert_eq!(
            args_for(&config, Some(&RenderOverrides::default())),
            config.args
        );
    }

    #[test]
    fn orientation_is_appended_so_the_last_flag_wins() {
        // f3d takes the last occurrence of a repeated option, so appending is
        // enough to override a global config that already sets `--up`.
        let config = RendererConfig {
            tool: "f3d".into(),
            args: vec!["{input}".into(), "--up=+Y".into()],
        };
        let args = args_for(&config, Some(&overrides(Some("+Z"), Some(45))));
        assert_eq!(
            args,
            vec!["{input}", "--up=+Y", "--up=+Z", "--camera-azimuth-angle=45"]
        );
    }

    #[test]
    fn a_full_turn_of_the_turntable_is_no_rotation() {
        // 0, 360 and -360 are the same picture: all of them store as "unset", so
        // stepping right around the circle doesn't leave the file looking
        // different from an untouched one.
        for degrees in [0, 360, -360, 720] {
            let normalised = overrides(None, Some(degrees)).normalise().unwrap();
            assert_eq!(normalised.turntable, None, "{degrees}° should be unset");
            assert!(normalised.is_empty());
        }
    }

    #[test]
    fn turning_the_wrong_way_past_zero_wraps_round() {
        assert_eq!(
            overrides(None, Some(-45)).normalise().unwrap().turntable,
            Some(315)
        );
        assert_eq!(
            overrides(None, Some(405)).normalise().unwrap().turntable,
            Some(45)
        );
    }

    #[test]
    fn every_axis_f3d_knows_is_accepted_and_nothing_else_is() {
        for axis in UP_AXES {
            assert!(overrides(Some(axis), None).normalise().is_ok(), "{axis}");
        }
        for bad in ["Z", "+z", "up", "+W", ""] {
            assert!(overrides(Some(bad), None).normalise().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn an_unset_orientation_serialises_to_nothing() {
        // The stored value is compared against the file's own for staleness, so
        // "no orientation" must not serialise to a `{"up":null}` that differs
        // from a plain absent one.
        let json = serde_json::to_value(RenderOverrides::default()).unwrap();
        assert_eq!(json, serde_json::json!({}));
        let json = serde_json::to_value(overrides(Some("+Z"), None)).unwrap();
        assert_eq!(json, serde_json::json!({ "up": "+Z" }));
    }
}
