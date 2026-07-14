use anyhow::{anyhow, Result};
use base64::Engine;

/// Capture the primary monitor and return a base64-encoded PNG.
///
/// Runs once per interval; between captures the agent is asleep, so the
/// (brief) cost here does not affect idle CPU.
pub fn capture_primary_png_base64() -> Result<String> {
    let monitors = xcap::Monitor::all()?;
    let primary = monitors
        .into_iter()
        .find(|m| m.is_primary())
        .ok_or_else(|| anyhow!("no primary monitor found"))?;

    let image = primary.capture_image()?;

    // Encode to PNG in-memory.
    let mut png_bytes: Vec<u8> = Vec::new();
    {
        use image::ImageEncoder;
        let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
        encoder.write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )?;
    }

    Ok(base64::engine::general_purpose::STANDARD.encode(png_bytes))
}
