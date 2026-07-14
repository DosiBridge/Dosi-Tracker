use anyhow::Result;
use base64::Engine;

/// Capture a single webcam frame and return it as a base64-encoded JPEG.
///
/// The camera is opened, one frame is grabbed, and it is released immediately
/// so the device is not held between intervals.
pub fn capture_jpg_base64() -> Result<String> {
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
    use nokhwa::Camera;

    let index = CameraIndex::Index(0);
    let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
    let mut camera = Camera::new(index, format)?;

    camera.open_stream()?;
    let frame = camera.frame()?;
    let decoded = frame.decode_image::<RgbFormat>()?;
    camera.stop_stream().ok();

    let mut jpg_bytes: Vec<u8> = Vec::new();
    {
        use image::ImageEncoder;
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg_bytes, 70);
        encoder.write_image(
            decoded.as_raw(),
            decoded.width(),
            decoded.height(),
            image::ExtendedColorType::Rgb8,
        )?;
    }

    Ok(base64::engine::general_purpose::STANDARD.encode(jpg_bytes))
}
