use anyhow::{anyhow, Result};
use base64::Engine;

/// Width of the list thumbnail sent alongside each screenshot. Small enough
/// that a page of dashboard cards loads in a blink (~15-40 KB each).
const THUMB_WIDTH: u32 = 480;
const THUMB_JPEG_QUALITY: u8 = 70;

/// One primary-monitor capture, base64-encoded for upload.
pub struct ScreenCapture {
    /// Full-resolution PNG (lossless, so text stays readable).
    pub png_base64: String,
    /// Small JPEG of the same frame. A thumbnail failure never costs the
    /// screenshot itself.
    pub thumb_jpg_base64: Option<String>,
}

/// Capture the primary monitor as a base64 PNG plus a JPEG thumbnail.
///
/// Runs once per interval; between captures the agent is asleep, so the
/// (brief) cost here does not affect idle CPU.
pub fn capture_primary() -> Result<ScreenCapture> {
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

    let thumb_jpg_base64 = match encode_thumbnail(&image) {
        Ok(jpeg) => Some(base64::engine::general_purpose::STANDARD.encode(jpeg)),
        Err(e) => {
            tracing::warn!(?e, "could not encode the screenshot thumbnail");
            None
        }
    };

    Ok(ScreenCapture {
        png_base64: base64::engine::general_purpose::STANDARD.encode(png_bytes),
        thumb_jpg_base64,
    })
}

/// Downscale to [`THUMB_WIDTH`] (never upscale) and encode as JPEG. JPEG has
/// no alpha channel, so the frame is flattened to RGB first.
fn encode_thumbnail(image: &image::RgbaImage) -> Result<Vec<u8>> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(anyhow!("empty capture"));
    }
    let thumb_width = THUMB_WIDTH.min(width);
    let thumb_height = ((u64::from(height) * u64::from(thumb_width)) / u64::from(width)).max(1) as u32;
    let thumb = image::imageops::thumbnail(image, thumb_width, thumb_height);
    let rgb = image::DynamicImage::ImageRgba8(thumb).into_rgb8();

    let mut jpeg = Vec::new();
    {
        use image::ImageEncoder;
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, THUMB_JPEG_QUALITY).write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )?;
    }
    Ok(jpeg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_is_a_downscaled_jpeg_with_the_same_aspect() {
        let frame = image::RgbaImage::from_pixel(1920, 1080, image::Rgba([40, 90, 200, 255]));
        let jpeg = encode_thumbnail(&frame).unwrap();
        let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (480, 270));
    }

    /// Needs a real desktop session: `cargo test -- --ignored`.
    #[test]
    #[ignore = "captures the real screen"]
    fn captures_the_primary_monitor_with_a_thumbnail() {
        let capture = capture_primary().unwrap();
        let decode = |b64: &str| base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let png = image::load_from_memory(&decode(&capture.png_base64)).unwrap();
        let thumb = image::load_from_memory(&decode(capture.thumb_jpg_base64.as_deref().unwrap())).unwrap();
        assert_eq!(thumb.width(), THUMB_WIDTH.min(png.width()));
        let thumb_kb = decode(capture.thumb_jpg_base64.as_deref().unwrap()).len() / 1024;
        let png_kb = decode(&capture.png_base64).len() / 1024;
        println!("screen {}x{}: png {png_kb} KB, thumb {}x{} {thumb_kb} KB", png.width(), png.height(), thumb.width(), thumb.height());
        assert!(super::super::active_window::current().is_some());
    }

    #[test]
    fn small_frames_are_not_upscaled() {
        let frame = image::RgbaImage::from_pixel(320, 200, image::Rgba([0, 0, 0, 255]));
        let jpeg = encode_thumbnail(&frame).unwrap();
        let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (320, 200));
        assert!(encode_thumbnail(&image::RgbaImage::new(0, 0)).is_err());
    }
}
