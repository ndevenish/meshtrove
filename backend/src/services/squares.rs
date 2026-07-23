//! Square previews for images that aren't square, by seam carving.
//!
//! Cards are a square grid, so a 16:9 photo has to lose a third of itself to
//! fit one. `object-fit: cover` takes that third off the ends, which is where
//! the subject usually isn't — but it is also where the *frame* is, and a
//! centre crop happily beheads a model that was photographed slightly off
//! centre. Seam carving takes the pixels back from wherever the picture is
//! least interesting instead: repeatedly remove the lowest-energy path from top
//! to bottom, so flat backdrop collapses and the printed thing keeps its
//! proportions.
//!
//! The result is a derived file, not a blob: it is a rendering decision we can
//! change our minds about, reproducible from bytes we still hold. It lives
//! beside the store in `<store>/squares/` rather than in it, and nothing in
//! Postgres points at it — deleting the whole directory costs nothing but the
//! CPU to carve it again.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::{DynamicImage, RgbaImage, imageops::FilterType};

/// Edge length a card asks for by default: 512 covers a ~210px grid cell on a
/// 2× display without carving four times the pixels anyone will see.
pub const DEFAULT_SIZE: u32 = 512;
pub const MIN_SIZE: u32 = 64;
pub const MAX_SIZE: u32 = 1024;

/// How much of the long side carving is allowed to take. Seam carving degrades
/// gracefully up to about half; past that it runs out of dull pixels and starts
/// eating the subject, so a panorama gets cropped down to this budget first and
/// carved the rest of the way. 16:9 (44%) — the shape most screenshots arrive
/// in — lands just inside it and is carved outright.
const MAX_CARVE_FRACTION: f32 = 0.45;

/// Aspect ratios this close to 1:1 are left alone: the couple of pixels a card
/// crops off them are not worth a re-encode, let alone a carve.
const SQUARE_TOLERANCE: f32 = 0.015;

pub fn is_effectively_square(width: u32, height: u32) -> bool {
    let (w, h) = (width.max(1) as f32, height.max(1) as f32);
    (w.max(h) / w.min(h)) - 1.0 <= SQUARE_TOLERANCE
}

/// A carved preview on disk, ready to stream.
pub struct SquarePreview {
    pub path: PathBuf,
    pub mime: &'static str,
}

fn squares_dir(store_dir: &Path) -> PathBuf {
    store_dir.join("squares")
}

/// `<store>/squares/ab/<sha256>-<size>.<ext>`. Fanned out on the first byte for
/// the same reason blobs are: one directory per library is a directory no tool
/// enjoys listing.
fn cache_path(store_dir: &Path, sha256: &str, size: u32, ext: &str) -> PathBuf {
    squares_dir(store_dir)
        .join(&sha256[0..2])
        .join(format!("{sha256}-{size}.{ext}"))
}

/// The already-carved preview for this blob, if there is one.
pub fn cached(store_dir: &Path, sha256: &str, size: u32) -> Option<SquarePreview> {
    for (ext, mime) in [("jpg", "image/jpeg"), ("png", "image/png")] {
        let path = cache_path(store_dir, sha256, size, ext);
        if path.exists() {
            return Some(SquarePreview { path, mime });
        }
    }
    None
}

/// Carve `source` square and cache the result. Blocking and CPU-bound — call it
/// from `spawn_blocking`.
///
/// `None` means the source is already square (within tolerance): there is
/// nothing to carve and nothing cached, so the caller should serve the original
/// blob rather than a re-encoded copy of it. Uploads carry no stored dimensions,
/// so this decode is the first place squareness is actually known.
pub fn build(
    store_dir: &Path,
    source: &Path,
    sha256: &str,
    size: u32,
) -> Result<Option<SquarePreview>> {
    let image = image::ImageReader::open(source)
        .with_context(|| format!("opening image blob {sha256}"))?
        .with_guessed_format()?
        .decode()
        .with_context(|| format!("decoding image blob {sha256}"))?;

    if is_effectively_square(image.width(), image.height()) {
        return Ok(None);
    }

    let square = carve_to_square(&image, size);

    // Alpha survives the carve, so it has to survive the encode: a PNG with a
    // cut-out background would come back from JPEG on a black one. Photos —
    // which is what anything non-square almost always is — take the JPEG.
    let opaque = square.pixels().all(|p| p.0[3] == 255);
    let (ext, mime) = if opaque {
        ("jpg", "image/jpeg")
    } else {
        ("png", "image/png")
    };
    let path = cache_path(store_dir, sha256, size, ext);
    std::fs::create_dir_all(path.parent().expect("cache path has parent"))?;

    // Same write-then-rename as the blob store: a reader either sees a whole
    // preview or no preview, never half of one.
    let tmp_dir = store_dir.join("tmp");
    std::fs::create_dir_all(&tmp_dir)?;
    let tmp = tmp_dir.join(format!("square-{}", uuid::Uuid::new_v4()));
    let write = || -> Result<()> {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        if opaque {
            let rgb = DynamicImage::ImageRgba8(square.clone()).into_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88).encode_image(&rgb)?;
        } else {
            square.write_to(&mut out, image::ImageFormat::Png)?;
        }
        Ok(())
    };
    if let Err(error) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    std::fs::rename(&tmp, &path)?;
    Ok(Some(SquarePreview { path, mime }))
}

/// Drop cached previews for blobs the store no longer holds. Nothing references
/// these files, so nothing deletes them when their source goes — this is where
/// that catches up. Returns the number of files removed (or that would be, on a
/// dry run).
pub fn purge_stale(
    store_dir: &Path,
    live: &std::collections::HashSet<String>,
    dry_run: bool,
) -> Result<u64> {
    let root = squares_dir(store_dir);
    let mut removed = 0;
    let level1 = match std::fs::read_dir(&root) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    for dir in level1 {
        let dir = dir?;
        if !dir.file_type()?.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(dir.path())? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(sha) = name.to_str().and_then(|n| n.split('-').next()) else {
                continue;
            };
            if live.contains(sha) {
                continue;
            }
            removed += 1;
            if !dry_run {
                std::fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(removed)
}

/// Reduce an image to a square of (at most) `size` a side.
///
/// Carving is done at display scale, not source scale: the picture is resized
/// so its *short* side is already the answer, and only then carved along the
/// long one. A 4000×2250 photo carved at full size would be 1750 seams over 9
/// megapixels for a thumbnail — the same seams found on a 910×512 copy cost a
/// hundredth of that and land in the same places, because the low-energy
/// regions a seam follows are the large ones.
pub fn carve_to_square(image: &DynamicImage, size: u32) -> RgbaImage {
    let (w, h) = (image.width().max(1), image.height().max(1));
    // Never upscale: a small image's own short side is the best square it has.
    let side = size.min(w.min(h));
    let (scaled_w, scaled_h) = if w >= h {
        (
            ((w as u64 * side as u64) / h as u64).max(side as u64) as u32,
            side,
        )
    } else {
        (
            side,
            ((h as u64 * side as u64) / w as u64).max(side as u64) as u32,
        )
    };
    let scaled = image
        .resize_exact(scaled_w, scaled_h, FilterType::Lanczos3)
        .to_rgba8();
    if scaled_w == scaled_h {
        return scaled;
    }

    let landscape = scaled_w > scaled_h;
    // Carving only knows how to remove vertical seams; a portrait is a
    // landscape lying down.
    let mut canvas = Canvas::from_image(&scaled);
    if !landscape {
        canvas = canvas.transposed();
    }

    // Past the carve budget, take the overflow off the ends first — a 3:1
    // panorama has to lose two thirds of itself, and no amount of seam-finding
    // makes that invisible.
    let budget = (canvas.w as f32 * MAX_CARVE_FRACTION) as usize;
    let excess = canvas.w - side as usize;
    if excess > budget {
        canvas.crop_centre(side as usize + budget);
    }
    while canvas.w > side as usize {
        canvas.remove_seam();
    }

    if !landscape {
        canvas = canvas.transposed();
    }
    canvas.into_image()
}

/// RGBA pixels with a fixed stride and a shrinking logical width. Removing a
/// seam shuffles each row left over the pixel it dropped rather than
/// reallocating the buffer, so 400 seams are 400 memmoves, not 400 images.
struct Canvas {
    px: Vec<[u8; 4]>,
    stride: usize,
    w: usize,
    h: usize,
    /// Scratch, reused every seam: per-pixel energy and the cumulative cost of
    /// the cheapest path reaching each pixel from the top.
    energy: Vec<f32>,
    cost: Vec<f32>,
}

impl Canvas {
    fn from_image(image: &RgbaImage) -> Canvas {
        let (w, h) = (image.width() as usize, image.height() as usize);
        let px = image.pixels().map(|p| p.0).collect();
        Canvas {
            px,
            stride: w,
            w,
            h,
            energy: vec![0.0; w * h],
            cost: vec![0.0; w * h],
        }
    }

    fn into_image(self) -> RgbaImage {
        let mut out = RgbaImage::new(self.w as u32, self.h as u32);
        for y in 0..self.h {
            let row = &self.px[y * self.stride..y * self.stride + self.w];
            for (x, p) in row.iter().enumerate() {
                out.put_pixel(x as u32, y as u32, image::Rgba(*p));
            }
        }
        out
    }

    fn transposed(&self) -> Canvas {
        let (w, h) = (self.h, self.w);
        let mut px = vec![[0u8; 4]; w * h];
        for y in 0..self.h {
            for x in 0..self.w {
                px[x * w + y] = self.px[y * self.stride + x];
            }
        }
        Canvas {
            px,
            stride: w,
            w,
            h,
            energy: vec![0.0; w * h],
            cost: vec![0.0; w * h],
        }
    }

    /// Keep the middle `width` columns.
    fn crop_centre(&mut self, width: usize) {
        let start = (self.w - width) / 2;
        for y in 0..self.h {
            let row = y * self.stride;
            self.px.copy_within(row + start..row + start + width, row);
        }
        self.w = width;
    }

    /// The gradient magnitude at each pixel: how much colour changes across it,
    /// horizontally and vertically. Edges score high, flat backdrop scores near
    /// zero, and a seam is a path that stays in the flat. Out-of-frame
    /// neighbours clamp to the edge pixel, which leaves a plain border cheap to
    /// remove — a frame is not a subject.
    fn compute_energy(&mut self) {
        for y in 0..self.h {
            let up = y.saturating_sub(1) * self.stride;
            let down = (y + 1).min(self.h - 1) * self.stride;
            let row = y * self.stride;
            for x in 0..self.w {
                let left = self.px[row + x.saturating_sub(1)];
                let right = self.px[row + (x + 1).min(self.w - 1)];
                let dx = squared_difference(left, right);
                let dy = squared_difference(self.px[up + x], self.px[down + x]);
                self.energy[row + x] = (dx + dy).sqrt();
            }
        }
    }

    /// Cheapest top-to-bottom path cost reaching each pixel, where a step may go
    /// straight down or one column either way — that connectedness is what makes
    /// a seam a seam rather than a scatter of cheap pixels.
    fn accumulate(&mut self) {
        self.cost[..self.w].copy_from_slice(&self.energy[..self.w]);
        for y in 1..self.h {
            let (above, row) = ((y - 1) * self.stride, y * self.stride);
            for x in 0..self.w {
                let mut best = self.cost[above + x];
                if x > 0 {
                    best = best.min(self.cost[above + x - 1]);
                }
                if x + 1 < self.w {
                    best = best.min(self.cost[above + x + 1]);
                }
                self.cost[row + x] = self.energy[row + x] + best;
            }
        }
    }

    /// Find the cheapest seam and close the gap behind it.
    fn remove_seam(&mut self) {
        self.compute_energy();
        self.accumulate();

        let last = (self.h - 1) * self.stride;
        let mut x = (0..self.w)
            .min_by(|&a, &b| self.cost[last + a].total_cmp(&self.cost[last + b]))
            .expect("a canvas being carved has at least one column");

        for y in (0..self.h).rev() {
            let row = y * self.stride;
            self.px.copy_within(row + x + 1..row + self.w, row + x);
            if y == 0 {
                break;
            }
            // Walk back up to whichever of the three pixels above fed this one.
            let above = (y - 1) * self.stride;
            let from = x.saturating_sub(1);
            let to = (x + 1).min(self.w - 1);
            x = (from..=to)
                .min_by(|&a, &b| self.cost[above + a].total_cmp(&self.cost[above + b]))
                .expect("the row above is non-empty");
        }
        self.w -= 1;
    }
}

fn squared_difference(a: [u8; 4], b: [u8; 4]) -> f32 {
    let mut total = 0.0;
    for channel in 0..3 {
        let delta = a[channel] as f32 - b[channel] as f32;
        total += delta * delta;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// A wide image whose only content is a textured vertical bar on a flat
    /// backdrop. The bar carries a smooth left-to-right brightness ramp so its
    /// *interior* has energy under the dual-gradient measure, not just its
    /// edges — a flat block, or a 1px alternation the central difference can't
    /// see, would be as cheap to carve as the backdrop and would collapse. That
    /// collapse is a true property of seam carving; the ramp stands in for the
    /// smooth gradients a real photographed subject actually has.
    fn bar_image(w: u32, h: u32, bar_x: u32, bar_w: u32) -> DynamicImage {
        let mut img = RgbaImage::from_pixel(w, h, Rgba([30, 30, 30, 255]));
        for y in 0..h {
            for x in bar_x..bar_x + bar_w {
                let v = (205 + 2 * (x - bar_x)).min(255) as u8;
                img.put_pixel(x, y, Rgba([v, v, v, 255]));
            }
        }
        DynamicImage::ImageRgba8(img)
    }

    fn count_bright_columns(img: &RgbaImage, y: u32) -> u32 {
        (0..img.width())
            .filter(|&x| img.get_pixel(x, y).0[0] > 200)
            .count() as u32
    }

    #[test]
    fn carving_a_landscape_yields_a_square() {
        let out = carve_to_square(&bar_image(320, 180, 150, 20), 180);
        assert_eq!((out.width(), out.height()), (180, 180));
    }

    #[test]
    fn carving_a_portrait_yields_a_square() {
        let out = carve_to_square(&bar_image(180, 320, 80, 20), 180);
        assert_eq!((out.width(), out.height()), (180, 180));
    }

    #[test]
    fn the_subject_survives_the_carve() {
        // 320→180 is a 44% cut, all of which should come out of the flat
        // surround rather than the bar. A centre crop would keep the bar too,
        // so the point of the assertion is the *width*: it is not squeezed.
        let out = carve_to_square(&bar_image(320, 180, 150, 20), 180);
        assert_eq!(count_bright_columns(&out, 90), 20);
    }

    #[test]
    fn an_extreme_panorama_is_cropped_before_it_is_carved() {
        // 6:1 is far past the carve budget, so it must still come out square
        // rather than looping forever or eating the subject entirely.
        let out = carve_to_square(&bar_image(1200, 200, 590, 20), 200);
        assert_eq!((out.width(), out.height()), (200, 200));
        assert_eq!(count_bright_columns(&out, 100), 20);
    }

    #[test]
    fn a_small_image_is_not_upscaled() {
        let out = carve_to_square(&bar_image(200, 120, 90, 10), 512);
        assert_eq!((out.width(), out.height()), (120, 120));
    }

    #[test]
    fn near_square_images_are_left_alone() {
        assert!(is_effectively_square(1000, 1000));
        assert!(is_effectively_square(1200, 1190));
        assert!(!is_effectively_square(1920, 1080));
        assert!(!is_effectively_square(1080, 1350));
    }

    #[test]
    fn alpha_survives_a_carve() {
        let mut img = RgbaImage::from_pixel(200, 100, Rgba([0, 0, 0, 0]));
        for y in 40..60 {
            for x in 90..110 {
                img.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        let out = carve_to_square(&DynamicImage::ImageRgba8(img), 100);
        assert_eq!((out.width(), out.height()), (100, 100));
        assert!(out.pixels().any(|p| p.0[3] == 0));
    }

    #[test]
    fn purge_keeps_live_previews_and_drops_the_rest() {
        let dir = std::env::temp_dir().join(format!("meshtrove-squares-{}", uuid::Uuid::new_v4()));
        let live_sha = "a".repeat(64);
        let dead_sha = "b".repeat(64);
        for sha in [&live_sha, &dead_sha] {
            let path = cache_path(&dir, sha, 512, "jpg");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"x").unwrap();
        }
        let live: std::collections::HashSet<String> = [live_sha.clone()].into_iter().collect();

        assert_eq!(purge_stale(&dir, &live, true).unwrap(), 1);
        assert!(cache_path(&dir, &dead_sha, 512, "jpg").exists());
        assert_eq!(purge_stale(&dir, &live, false).unwrap(), 1);
        assert!(!cache_path(&dir, &dead_sha, 512, "jpg").exists());
        assert!(cache_path(&dir, &live_sha, 512, "jpg").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn purging_a_store_that_never_carved_anything_is_fine() {
        let dir = std::env::temp_dir().join(format!("meshtrove-squares-{}", uuid::Uuid::new_v4()));
        assert_eq!(purge_stale(&dir, &Default::default(), false).unwrap(), 0);
    }
}
