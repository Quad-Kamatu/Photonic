//! Local, offline **background removal** (image matting).
//!
//! Runs the small, permissively-licensed **U²-Net-p** salient-object model
//! (~4.7 MB, Apache-2.0) fully on-device via `ort` (ONNX Runtime) — the same
//! runtime already used by [`photonic-embed`]. The model is downloaded once to a
//! per-user cache on first use, then works offline. No external AI service is
//! required.
//!
//! The public entry point [`remove_background`] takes a [`RasterImage`] and
//! returns a [`Mask`] — an 8-bit **foreground matte** (255 = subject, 0 =
//! background) sized to the input image. Callers apply it as a non-destructive
//! layer mask (see `RasterNode::set_foreground_matte`), so the operation is
//! fully reversible.

use anyhow::{anyhow, Context, Result};
use ndarray::Array4;
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use photonic_core::raster::{image::RasterImage, mask::Mask};
use std::path::PathBuf;
use std::sync::OnceLock;

/// U²-Net-p ONNX model, hosted on the `rembg` GitHub release. Overridable via the
/// `PHOTONIC_RMBG_MODEL_URL` environment variable.
const DEFAULT_MODEL_URL: &str =
    "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx";

/// The square side the U²-Net family expects at its input.
const INPUT_SIZE: u32 = 320;

/// ImageNet normalization constants used by the U²-Net preprocessing.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Stable per-user cache directory for the model, mirroring `photonic-embed`:
/// `$XDG_CACHE_HOME/Photonic/models`, `%APPDATA%\Photonic\models`, or
/// `~/.cache/Photonic/models`.
fn model_cache_dir() -> PathBuf {
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("APPDATA").ok().map(PathBuf::from))
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".cache"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Photonic").join("models")
}

/// Resolve the local model path, downloading it once if it is not already
/// cached. Honors `PHOTONIC_RMBG_MODEL_PATH` (use an existing local file) and
/// `PHOTONIC_RMBG_MODEL_URL` (override the download source).
fn ensure_model() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("PHOTONIC_RMBG_MODEL_PATH") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Ok(path);
        }
        return Err(anyhow!(
            "PHOTONIC_RMBG_MODEL_PATH points at a missing file: {}",
            path.display()
        ));
    }

    let dir = model_cache_dir();
    let path = dir.join("u2netp.onnx");
    // A previous partial download can leave a tiny truncated file that ort will
    // fail to parse; treat anything implausibly small as absent.
    let good = std::fs::metadata(&path)
        .map(|m| m.len() > 1_000_000)
        .unwrap_or(false);
    if good {
        return Ok(path);
    }

    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating model cache dir {}", dir.display()))?;
    let url =
        std::env::var("PHOTONIC_RMBG_MODEL_URL").unwrap_or_else(|_| DEFAULT_MODEL_URL.to_string());
    tracing::info!(url = %url, "downloading background-removal model (once)");

    let resp = ureq::get(&url)
        .call()
        .with_context(|| format!("downloading matting model from {url}"))?;
    let mut bytes: Vec<u8> = Vec::new();
    std::io::copy(&mut resp.into_reader(), &mut bytes).context("reading model response body")?;
    if bytes.len() < 1_000_000 {
        return Err(anyhow!(
            "downloaded model is implausibly small ({} bytes) — the source URL may be stale; \
             set PHOTONIC_RMBG_MODEL_URL or PHOTONIC_RMBG_MODEL_PATH",
            bytes.len()
        ));
    }

    // Write atomically via a temp file so a crash mid-write can't leave a
    // half-written model that later reads treat as valid.
    let tmp = path.with_extension("onnx.part");
    std::fs::write(&tmp, &bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("finalizing {}", path.display()))?;
    tracing::info!(path = %path.display(), bytes = bytes.len(), "matting model ready");
    Ok(path)
}

/// Process-wide cached session. Loading the model + spinning up an ort session is
/// slow (hundreds of ms); reuse it across calls. Held behind a `OnceLock` of a
/// `Result` string so a load failure is reported on every call without retrying a
/// broken model each time within a run.
fn session() -> Result<&'static Session> {
    static SESSION: OnceLock<std::result::Result<Session, String>> = OnceLock::new();
    SESSION
        .get_or_init(|| build_session().map_err(|e| format!("{e:#}")))
        .as_ref()
        .map_err(|e| anyhow!(e.clone()))
}

fn build_session() -> Result<Session> {
    let path = ensure_model()?;
    let session = Session::builder()
        .context("creating ort session builder")?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .context("setting optimization level")?
        .commit_from_file(&path)
        .with_context(|| format!("loading ONNX model {}", path.display()))?;
    Ok(session)
}

/// Whether the background-removal model is already downloaded (so the UI can hint
/// that the first click will pull ~4.7 MB).
pub fn model_is_cached() -> bool {
    if let Ok(p) = std::env::var("PHOTONIC_RMBG_MODEL_PATH") {
        return PathBuf::from(p).is_file();
    }
    std::fs::metadata(model_cache_dir().join("u2netp.onnx"))
        .map(|m| m.len() > 1_000_000)
        .unwrap_or(false)
}

/// Compute a **foreground matte** for `img`: a [`Mask`] the same size as `img`
/// with coverage `255` over the salient subject and `0` over the background.
///
/// Slow (model load on first call, then inference); run off the UI thread.
pub fn remove_background(img: &RasterImage) -> Result<Mask> {
    if img.width == 0 || img.height == 0 {
        return Err(anyhow!("cannot matte a zero-size image"));
    }
    let session = session()?;

    // ── Preprocess: resize to 320×320, normalize to CHW f32 ──────────────────
    let small = resize_rgba(img, INPUT_SIZE, INPUT_SIZE);
    // U²-Net divides by the max pixel value (not a fixed 255) before normalizing.
    let max = small.iter().copied().max().unwrap_or(255).max(1) as f32;
    let mut input = Array4::<f32>::zeros((1, 3, INPUT_SIZE as usize, INPUT_SIZE as usize));
    for y in 0..INPUT_SIZE as usize {
        for x in 0..INPUT_SIZE as usize {
            let i = (y * INPUT_SIZE as usize + x) * 4;
            for c in 0..3 {
                let v = small[i + c] as f32 / max;
                input[[0, c, y, x]] = (v - MEAN[c]) / STD[c];
            }
        }
    }

    // ── Run inference ────────────────────────────────────────────────────────
    let input_name = session.inputs[0].name.clone();
    let output_name = session.outputs[0].name.clone();
    let tensor = Tensor::from_array(input).context("building input tensor")?;
    let outputs = session
        .run(ort::inputs![input_name.as_str() => tensor]?)
        .context("running matting inference")?;
    let output = outputs[output_name.as_str()]
        .try_extract_tensor::<f32>()
        .context("extracting matting output")?;
    // `output` is an ndarray view of shape [1, 1, 320, 320]; flatten in logical
    // (row-major) order.
    let data: Vec<f32> = output.iter().copied().collect();

    // Output is a 320×320 saliency map. Normalize (pred-min)/(max-min) → 0..1.
    let n = (INPUT_SIZE * INPUT_SIZE) as usize;
    if data.len() < n {
        return Err(anyhow!(
            "matting output smaller than expected ({} < {})",
            data.len(),
            n
        ));
    }
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for &v in &data[..n] {
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    let range = (hi - lo).max(1e-6);

    // Build a 320×320 gray matte, then resize it up to the source dimensions.
    let mut matte_small = vec![0u8; n];
    for (dst, &v) in matte_small.iter_mut().zip(&data[..n]) {
        let t = ((v - lo) / range).clamp(0.0, 1.0);
        *dst = (t * 255.0).round() as u8;
    }
    let matte = resize_gray(&matte_small, INPUT_SIZE, INPUT_SIZE, img.width, img.height);
    Ok(Mask {
        width: img.width,
        height: img.height,
        data: matte,
    })
}

/// High-quality (Lanczos3) resize of an RGBA image to `w×h`, returning raw RGBA
/// bytes.
fn resize_rgba(img: &RasterImage, w: u32, h: u32) -> Vec<u8> {
    let buf: image::RgbaImage =
        match image::ImageBuffer::from_raw(img.width, img.height, img.pixels.clone()) {
            Some(b) => b,
            None => return vec![0u8; (w as usize) * (h as usize) * 4],
        };
    let resized = image::imageops::resize(&buf, w, h, image::imageops::FilterType::Lanczos3);
    resized.into_raw()
}

/// High-quality (Lanczos3) resize of a single-channel gray buffer.
fn resize_gray(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let buf: image::GrayImage = match image::ImageBuffer::from_raw(sw, sh, src.to_vec()) {
        Some(b) => b,
        None => return vec![0u8; (dw as usize) * (dh as usize)],
    };
    let resized = image::imageops::resize(&buf, dw, dh, image::imageops::FilterType::Lanczos3);
    resized.into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end: downloads the model on first run, then mattes a synthetic
    /// image (bright square on a dark field). Network + ~4.7 MB download, so it
    /// is `#[ignore]`d by default. Run with:
    ///   cargo test -p photonic-matte -- --ignored --nocapture
    #[test]
    #[ignore]
    fn matte_synthetic_subject() {
        let mut img = RasterImage::filled(96, 96, [12, 12, 20, 255]);
        for y in 30..66 {
            for x in 30..66 {
                img.set_pixel(x, y, [230, 220, 60, 255]);
            }
        }
        let mask = remove_background(&img).expect("matte");
        assert_eq!((mask.width, mask.height), (96, 96));
        let center = mask.get(48, 48) as u32;
        let corner = mask.get(3, 3) as u32;
        eprintln!("center={center} corner={corner}");
        assert!(
            center > corner + 40,
            "subject center ({center}) should score well above background corner ({corner})"
        );
    }
}
