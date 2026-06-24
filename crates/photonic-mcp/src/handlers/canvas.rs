use crate::protocol::{ScreenshotArgs, ToolResult};
use crate::server::AppState;
use base64::{engine::general_purpose, Engine};
use tokio::sync::oneshot;

pub async fn screenshot(state: &AppState, args: ScreenshotArgs) -> ToolResult {
    tracing::debug!("tool: screenshot — sending to render thread");
    let (tx, rx) = oneshot::channel::<Vec<u8>>();

    // Send the oneshot sender to the render thread via std::sync::mpsc
    let sent = state
        .capture_tx
        .lock()
        .map(|tx_guard| tx_guard.send(tx).is_ok())
        .unwrap_or(false);

    if !sent {
        return ToolResult::text(
            "Screenshot unavailable — render thread not running (use --headless for MCP-only mode)",
        );
    }

    match rx.await {
        Ok(png_bytes) if !png_bytes.is_empty() => {
            tracing::debug!("tool: screenshot — received {} bytes", png_bytes.len());

            // Downscale if requested (reduces base64 size significantly)
            let final_bytes = if let Some(scale) = args.scale {
                if scale > 0.0 && scale < 1.0 {
                    downscale_png(&png_bytes, scale).unwrap_or(png_bytes)
                } else {
                    png_bytes
                }
            } else {
                png_bytes
            };

            let encoded = general_purpose::STANDARD.encode(&final_bytes);
            ToolResult::text(format!("Screenshot captured ({} bytes)", final_bytes.len()))
                .with_image(encoded)
        }
        other => {
            tracing::warn!(
                "tool: screenshot — render thread did not respond: {:?}",
                other.err()
            );
            ToolResult::error("Render thread did not return a screenshot")
        }
    }
}

/// Decode a PNG, resize by `scale`, re-encode to PNG.
fn downscale_png(png_bytes: &[u8], scale: f32) -> Option<Vec<u8>> {
    use image::{imageops::FilterType, ImageFormat};
    let img = image::load_from_memory_with_format(png_bytes, ImageFormat::Png).ok()?;
    let new_w = ((img.width() as f32 * scale).round() as u32).max(1);
    let new_h = ((img.height() as f32 * scale).round() as u32).max(1);
    let resized = img.resize_exact(new_w, new_h, FilterType::Triangle);
    let mut out: Vec<u8> = Vec::new();
    resized
        .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
        .ok()?;
    Some(out)
}
