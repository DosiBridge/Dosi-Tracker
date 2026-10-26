//! Screenshot encoding ported from Windows: JPEG ladder within 8 MB, 480 px
//! thumbnail within 512 KB, blur when required.
//!
//! The screen sources ([`super::x11_screen`], [`super::portal_screen`]) only
//! produce a [`RawFrame`]; this module turns it into what is queued: a JPEG at
//! most [`MAX_SCREEN_WIDTH`] wide, optionally blurred so no text can be read,
//! plus a thumbnail of the same frame. Captures are kept as bytes (the outbox
//! stores them as BLOBs); base64 is added only at upload.

use chrono::{DateTime, Utc};
use image::imageops::{self, FilterType};
use image::RgbImage;

use super::traits::TrackingError;
use crate::outbox::{CaptureKind, PendingCapture};

/// Widest screen capture sent: a 4K screen is halved, which keeps text
/// legible while a capture stays a few hundred KB.
pub const MAX_SCREEN_WIDTH: u32 = 1920;
const SCREEN_JPEG_QUALITY: u8 = 80;

/// Width of the list thumbnail sent alongside each screenshot. Small enough
/// that a page of dashboard cards loads in a blink (~15-40 KB each).
pub const THUMB_WIDTH: u32 = 480;
const THUMB_JPEG_QUALITY: u8 = 70;

/// The server's limits on the decoded images: a capture over them is
/// dropped there, after the upload — so it is never sent.
pub const MAX_SCREEN_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_THUMB_BYTES: usize = 512 * 1024;

/// Qualities tried, in order, until a capture fits its limit.
const FALLBACK_QUALITIES: [u8; 3] = [60, 45, 30];

/// The blur first averages blocks this many pixels wide — about two
/// characters of ordinary UI text...
const BLUR_CELL: u32 = 16;

/// ...then spreads each block over its neighbours (a Gaussian, this many
/// blocks wide), so every pixel of the result mixes several characters of
/// several lines and no letter, or its outline, survives.
const BLUR_SPREAD_CELLS: f32 = 1.5;

/// Largest side accepted from a decoded portal PNG (bounds memory against a
/// hostile or broken file).
pub const MAX_DECODED_SIDE: u32 = 16_384;

/// Largest portal PNG file read, in bytes.
pub const MAX_PNG_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// One frame from a screen source, before encoding.
#[derive(Clone)]
pub struct RawFrame {
    pub image: RgbImage,
    /// When the screen was captured.
    pub captured_at: DateTime<Utc>,
    /// The monitor the frame shows, when the source knows it.
    pub monitor: Option<MonitorTarget>,
}

impl std::fmt::Debug for RawFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawFrame")
            .field("width", &self.image.width())
            .field("height", &self.image.height())
            .field("captured_at", &self.captured_at)
            .field("monitor", &self.monitor)
            .finish()
    }
}

/// One encoded screen capture, as queued.
#[derive(Clone, PartialEq, Eq)]
pub struct ScreenCapture {
    /// JPEG of the screen, at most [`MAX_SCREEN_WIDTH`] wide and
    /// [`MAX_SCREEN_BYTES`] long.
    pub jpeg: Vec<u8>,
    /// Small JPEG of the same frame. A thumbnail failure never costs the
    /// screenshot itself.
    pub thumb_jpeg: Option<Vec<u8>>,
    /// When the screen was captured.
    pub captured_at: DateTime<Utc>,
    /// The screen and the thumbnail were blurred.
    pub blurred: bool,
}

impl std::fmt::Debug for ScreenCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenCapture")
            .field("jpeg_bytes", &self.jpeg.len())
            .field("thumb_bytes", &self.thumb_jpeg.as_ref().map(Vec::len))
            .field("captured_at", &self.captured_at)
            .field("blurred", &self.blurred)
            .finish()
    }
}

impl ScreenCapture {
    /// The capture as outbox rows (screen first, then the thumbnail).
    pub fn pending_captures(&self) -> Vec<PendingCapture> {
        let mut out = vec![PendingCapture {
            kind: CaptureKind::Screen,
            jpeg: self.jpeg.clone(),
            captured_at: self.captured_at,
            blurred: self.blurred,
        }];
        if let Some(thumb) = &self.thumb_jpeg {
            out.push(PendingCapture {
                kind: CaptureKind::Thumb,
                jpeg: thumb.clone(),
                captured_at: self.captured_at,
                blurred: self.blurred,
            });
        }
        out
    }
}

/// A monitor as a screen source lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorRef {
    /// Source-specific id (on X11 the RandR monitor's index).
    pub id: u32,
    /// Monitor name, e.g. `DP-1`.
    pub name: String,
    pub primary: bool,
}

/// The monitor the active window is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorTarget {
    pub id: u32,
    pub name: String,
}

/// Which monitor to capture: the target (by id, then by name), else the
/// primary, else the first.
pub fn pick_monitor(monitors: &[MonitorRef], target: Option<&MonitorTarget>) -> Option<usize> {
    target
        .and_then(|t| {
            monitors.iter().position(|m| m.id == t.id).or_else(|| {
                monitors
                    .iter()
                    .position(|m| !t.name.is_empty() && m.name.eq_ignore_ascii_case(&t.name))
            })
        })
        .or_else(|| monitors.iter().position(|m| m.primary))
        .or(if monitors.is_empty() { None } else { Some(0) })
}

/// [`encode_capture`] of a source's frame.
pub fn encode_frame(frame: RawFrame, blur: bool) -> Result<ScreenCapture, TrackingError> {
    encode_capture(frame.image, blur, frame.captured_at)
}

/// Turn a captured frame into what is uploaded: downscaled to at most
/// [`MAX_SCREEN_WIDTH`], blurred if asked, a JPEG within
/// [`MAX_SCREEN_BYTES`], and a thumbnail of the same (blurred) frame.
pub fn encode_capture(
    frame: RgbImage,
    blur: bool,
    captured_at: DateTime<Utc>,
) -> Result<ScreenCapture, TrackingError> {
    if frame.width() == 0 || frame.height() == 0 {
        return Err(TrackingError::EncodeError("empty capture".into()));
    }
    let frame = fit_width(frame, MAX_SCREEN_WIDTH);
    let frame = if blur { blur_unreadable(&frame) } else { frame };
    let jpeg = encode_jpeg_within(&frame, SCREEN_JPEG_QUALITY, MAX_SCREEN_BYTES)?;

    let thumb_jpeg = match encode_thumbnail(&frame) {
        Ok(thumb) => Some(thumb),
        Err(e) => {
            tracing::warn!(error = %e, "could not encode the screenshot thumbnail");
            None
        }
    };

    Ok(ScreenCapture {
        jpeg,
        thumb_jpeg,
        captured_at,
        blurred: blur,
    })
}

/// Decode a PNG (the Screenshot portal's file) into a frame, refusing images
/// larger than [`MAX_DECODED_SIDE`] on a side.
pub fn decode_png(bytes: &[u8]) -> Result<RgbImage, TrackingError> {
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODED_SIDE);
    limits.max_image_height = Some(MAX_DECODED_SIDE);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|e| TrackingError::EncodeError(format!("unreadable PNG: {e}")))?;
    Ok(image.into_rgb8())
}

/// Scale down to at most `max_width` wide, keeping the aspect (never up).
pub fn fit_width(frame: RgbImage, max_width: u32) -> RgbImage {
    let (width, height) = frame.dimensions();
    if width <= max_width {
        return frame;
    }
    let new_height = ((u64::from(height) * u64::from(max_width)) / u64::from(width)).max(1) as u32;
    imageops::resize(&frame, max_width, new_height, FilterType::Triangle)
}

/// Blur so strongly that no text can be read: every [`BLUR_CELL`]-pixel
/// block is averaged to one pixel, those pixels are blurred into each other
/// ([`BLUR_SPREAD_CELLS`]), and the result is scaled back up smoothly. Window
/// layout and colours stay recognisable; what remains of text is where it is
/// and how dense it is, never the letters.
///
/// Working on the reduced image keeps it cheap (a 1920×1080 frame is blurred
/// as 120×68 pixels).
pub fn blur_unreadable(frame: &RgbImage) -> RgbImage {
    let (width, height) = frame.dimensions();
    let small = imageops::resize(
        frame,
        width.div_ceil(BLUR_CELL).max(1),
        height.div_ceil(BLUR_CELL).max(1),
        FilterType::Triangle,
    );
    let small = imageops::blur(&small, BLUR_SPREAD_CELLS);
    imageops::resize(&small, width, height, FilterType::Triangle)
}

/// JPEG-encode `frame` at `quality`, stepping the quality down until it fits
/// in `max_bytes`; an error if even the lowest does not.
pub fn encode_jpeg_within(
    frame: &RgbImage,
    quality: u8,
    max_bytes: usize,
) -> Result<Vec<u8>, TrackingError> {
    for quality in
        std::iter::once(quality).chain(FALLBACK_QUALITIES.into_iter().filter(|&q| q < quality))
    {
        let jpeg = encode_jpeg(frame, quality)?;
        if jpeg.len() <= max_bytes {
            return Ok(jpeg);
        }
    }
    Err(TrackingError::EncodeError(format!(
        "capture larger than {max_bytes} bytes even at low quality"
    )))
}

/// JPEG-encode `frame` at `quality`.
pub fn encode_jpeg(frame: &RgbImage, quality: u8) -> Result<Vec<u8>, TrackingError> {
    use image::ImageEncoder;
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality)
        .write_image(
            frame.as_raw(),
            frame.width(),
            frame.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| TrackingError::EncodeError(format!("JPEG encoding failed: {e}")))?;
    Ok(jpeg)
}

/// Downscale to [`THUMB_WIDTH`] (never upscale) and encode as JPEG within
/// [`MAX_THUMB_BYTES`].
pub fn encode_thumbnail(frame: &RgbImage) -> Result<Vec<u8>, TrackingError> {
    let (width, height) = frame.dimensions();
    if width == 0 || height == 0 {
        return Err(TrackingError::EncodeError("empty capture".into()));
    }
    let thumb_width = THUMB_WIDTH.min(width);
    let thumb_height =
        ((u64::from(height) * u64::from(thumb_width)) / u64::from(width)).max(1) as u32;
    let thumb = imageops::thumbnail(frame, thumb_width, thumb_height);
    encode_jpeg_within(&thumb, THUMB_JPEG_QUALITY, MAX_THUMB_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    #[test]
    fn the_active_monitor_is_matched_by_id_then_name_then_primary() {
        let monitors = vec![
            MonitorRef {
                id: 0,
                name: "DP-1".into(),
                primary: true,
            },
            MonitorRef {
                id: 1,
                name: "HDMI-1".into(),
                primary: false,
            },
        ];
        let target = |id: u32, name: &str| MonitorTarget {
            id,
            name: name.into(),
        };
        assert_eq!(pick_monitor(&monitors, Some(&target(1, ""))), Some(1));
        assert_eq!(pick_monitor(&monitors, Some(&target(9, "hdmi-1"))), Some(1));
        assert_eq!(
            pick_monitor(&monitors, Some(&target(9, "DP-9"))),
            Some(0),
            "unknown: the primary"
        );
        assert_eq!(pick_monitor(&monitors, None), Some(0));
        let no_primary = vec![MonitorRef {
            primary: false,
            ..monitors[1].clone()
        }];
        assert_eq!(pick_monitor(&no_primary, None), Some(0));
        assert_eq!(pick_monitor(&[], None), None);
    }

    #[test]
    fn a_capture_becomes_screen_and_thumb_rows() {
        let at = Utc::now();
        let capture = encode_capture(
            RgbImage::from_pixel(64, 48, image::Rgb([1, 2, 3])),
            true,
            at,
        )
        .unwrap();
        let rows = capture.pending_captures();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, CaptureKind::Screen);
        assert_eq!(rows[0].jpeg, capture.jpeg);
        assert_eq!(rows[1].kind, CaptureKind::Thumb);
        assert!(rows.iter().all(|r| r.blurred && r.captured_at == at));
        // Debug never dumps the image bytes.
        assert!(!format!("{capture:?}").contains("255, 216"));
    }

    #[test]
    fn png_bombs_and_garbage_are_refused() {
        assert!(matches!(
            decode_png(b"not a png"),
            Err(TrackingError::EncodeError(_))
        ));
        // A valid PNG one pixel wider than accepted (cheap: one row).
        let wide = RgbImage::new(MAX_DECODED_SIDE + 1, 1);
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                wide.as_raw(),
                wide.width(),
                wide.height(),
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        assert!(decode_png(&png).is_err());
        // The same at the limit decodes.
        let ok = RgbImage::new(MAX_DECODED_SIDE, 1);
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(ok.as_raw(), ok.width(), 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        assert_eq!(decode_png(&png).unwrap().width(), MAX_DECODED_SIDE);
    }
}
