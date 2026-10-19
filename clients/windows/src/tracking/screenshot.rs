//! Screen captures: the monitor holding the foreground window, as a JPEG at
//! most 1920 px wide plus a small list thumbnail — optionally blurred so no
//! text can be read.

use anyhow::{anyhow, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use image::imageops::{self, FilterType};
use image::RgbImage;

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

/// One screen capture, base64-encoded for upload.
#[derive(Debug, Clone)]
pub struct ScreenCapture {
    /// JPEG of the screen, at most [`MAX_SCREEN_WIDTH`] wide.
    pub jpeg_base64: String,
    /// Small JPEG of the same frame. A thumbnail failure never costs the
    /// screenshot itself.
    pub thumb_jpg_base64: Option<String>,
    /// When the screen was captured.
    pub captured_at: DateTime<Utc>,
    /// The screen and the thumbnail were blurred.
    pub blurred: bool,
}

/// Capture the monitor that holds the foreground window (the primary one
/// when that cannot be told) and encode it for upload.
///
/// Runs once per block; between captures the agent is asleep, so the (brief)
/// cost here does not affect idle CPU.
pub fn capture_foreground_monitor(blur: bool) -> Result<ScreenCapture> {
    let monitors = xcap::Monitor::all()?;
    let candidates: Vec<MonitorRef> = monitors
        .iter()
        .map(|m| MonitorRef { id: m.id(), name: m.name().to_string(), primary: m.is_primary() })
        .collect();
    let index = pick_monitor(&candidates, foreground_monitor().as_ref()).ok_or_else(|| anyhow!("no monitor to capture"))?;
    let captured_at = Utc::now();
    let frame = monitors[index].capture_image()?;
    encode_capture(image::DynamicImage::ImageRgba8(frame).into_rgb8(), blur, captured_at)
}

/// A monitor as xcap lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorRef {
    /// xcap's id: the `HMONITOR` value.
    pub id: u32,
    /// Device name, e.g. `\\.\DISPLAY2`.
    pub name: String,
    pub primary: bool,
}

/// The monitor the foreground window is on, as Windows names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorTarget {
    pub id: u32,
    pub name: String,
}

/// Which monitor to capture: the target (by handle, then by device name),
/// else the primary, else the first.
pub fn pick_monitor(monitors: &[MonitorRef], target: Option<&MonitorTarget>) -> Option<usize> {
    target
        .and_then(|t| {
            monitors
                .iter()
                .position(|m| m.id == t.id)
                .or_else(|| monitors.iter().position(|m| !t.name.is_empty() && m.name.eq_ignore_ascii_case(&t.name)))
        })
        .or_else(|| monitors.iter().position(|m| m.primary))
        .or(if monitors.is_empty() { None } else { Some(0) })
}

#[cfg(windows)]
fn foreground_monitor() -> Option<MonitorTarget> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONULL};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
        if monitor.is_invalid() {
            return None;
        }
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        let name = if GetMonitorInfoW(monitor, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
            let len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
            String::from_utf16_lossy(&info.szDevice[..len])
        } else {
            String::new()
        };
        Some(MonitorTarget { id: monitor.0 as usize as u32, name })
    }
}

#[cfg(not(windows))]
fn foreground_monitor() -> Option<MonitorTarget> {
    None
}

/// Turn a captured frame into what is uploaded: downscaled to at most
/// [`MAX_SCREEN_WIDTH`], blurred if asked, a JPEG within
/// [`MAX_SCREEN_BYTES`], and a thumbnail of the same (blurred) frame.
pub fn encode_capture(frame: RgbImage, blur: bool, captured_at: DateTime<Utc>) -> Result<ScreenCapture> {
    if frame.width() == 0 || frame.height() == 0 {
        return Err(anyhow!("empty capture"));
    }
    let frame = fit_width(frame, MAX_SCREEN_WIDTH);
    let frame = if blur { blur_unreadable(&frame) } else { frame };
    let jpeg = encode_jpeg_within(&frame, SCREEN_JPEG_QUALITY, MAX_SCREEN_BYTES)?;

    let thumb_jpg_base64 = match encode_thumbnail(&frame) {
        Ok(thumb) => Some(base64::engine::general_purpose::STANDARD.encode(thumb)),
        Err(e) => {
            tracing::warn!(?e, "could not encode the screenshot thumbnail");
            None
        }
    };

    Ok(ScreenCapture {
        jpeg_base64: base64::engine::general_purpose::STANDARD.encode(jpeg),
        thumb_jpg_base64,
        captured_at,
        blurred: blur,
    })
}

/// Scale down to at most `max_width` wide, keeping the aspect (never up).
fn fit_width(frame: RgbImage, max_width: u32) -> RgbImage {
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
fn encode_jpeg_within(frame: &RgbImage, quality: u8, max_bytes: usize) -> Result<Vec<u8>> {
    for quality in std::iter::once(quality).chain(FALLBACK_QUALITIES.into_iter().filter(|&q| q < quality)) {
        let jpeg = encode_jpeg(frame, quality)?;
        if jpeg.len() <= max_bytes {
            return Ok(jpeg);
        }
    }
    Err(anyhow!("capture larger than {max_bytes} bytes even at low quality"))
}

fn encode_jpeg(frame: &RgbImage, quality: u8) -> Result<Vec<u8>> {
    use image::ImageEncoder;
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality).write_image(
        frame.as_raw(),
        frame.width(),
        frame.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(jpeg)
}

/// Downscale to [`THUMB_WIDTH`] (never upscale) and encode as JPEG within
/// [`MAX_THUMB_BYTES`].
fn encode_thumbnail(frame: &RgbImage) -> Result<Vec<u8>> {
    let (width, height) = frame.dimensions();
    if width == 0 || height == 0 {
        return Err(anyhow!("empty capture"));
    }
    let thumb_width = THUMB_WIDTH.min(width);
    let thumb_height = ((u64::from(height) * u64::from(thumb_width)) / u64::from(width)).max(1) as u32;
    let thumb = imageops::thumbnail(frame, thumb_width, thumb_height);
    encode_jpeg_within(&thumb, THUMB_JPEG_QUALITY, MAX_THUMB_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(b64: &str) -> image::DynamicImage {
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        // JPEG magic: the server detects the format from it.
        assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF]);
        image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).unwrap()
    }

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn thumbnail_is_a_downscaled_jpeg_with_the_same_aspect() {
        let frame = RgbImage::from_pixel(1920, 1080, image::Rgb([40, 90, 200]));
        let jpeg = encode_thumbnail(&frame).unwrap();
        let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (480, 270));
    }

    #[test]
    fn small_frames_are_not_upscaled() {
        let frame = RgbImage::from_pixel(320, 200, image::Rgb([0, 0, 0]));
        let jpeg = encode_thumbnail(&frame).unwrap();
        let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (320, 200));
        assert!(encode_thumbnail(&RgbImage::new(0, 0)).is_err());
    }

    #[test]
    fn a_4k_screen_is_sent_as_a_1920_wide_jpeg_with_a_thumbnail() {
        let at = now();
        let frame = RgbImage::from_fn(3840, 2160, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
        let capture = encode_capture(frame, false, at).unwrap();
        let screen = decode(&capture.jpeg_base64);
        assert_eq!((screen.width(), screen.height()), (1920, 1080));
        let thumb = decode(capture.thumb_jpg_base64.as_deref().unwrap());
        assert_eq!((thumb.width(), thumb.height()), (480, 270));
        assert_eq!(capture.captured_at, at);
        assert!(!capture.blurred);
        // Far below the server's limit.
        assert!(base64::engine::general_purpose::STANDARD.decode(&capture.jpeg_base64).unwrap().len() < MAX_SCREEN_BYTES / 4);
    }

    #[test]
    fn narrow_screens_keep_their_size() {
        let capture = encode_capture(RgbImage::from_pixel(1366, 768, image::Rgb([200, 200, 200])), false, now()).unwrap();
        let screen = decode(&capture.jpeg_base64);
        assert_eq!((screen.width(), screen.height()), (1366, 768));
        assert!(encode_capture(RgbImage::new(0, 0), false, now()).is_err());
    }

    #[test]
    fn oversized_captures_step_down_in_quality_or_are_refused() {
        // Noise compresses worst.
        let mut seed: u32 = 12345;
        let noise = RgbImage::from_fn(640, 480, |_, _| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let v = (seed >> 16) as u8;
            image::Rgb([v, v.wrapping_mul(7), v.wrapping_mul(13)])
        });
        let full = encode_jpeg(&noise, SCREEN_JPEG_QUALITY).unwrap().len();
        let fitted = encode_jpeg_within(&noise, SCREEN_JPEG_QUALITY, full - 1).unwrap();
        assert!(fitted.len() < full, "a lower quality was used");
        assert!(encode_jpeg_within(&noise, SCREEN_JPEG_QUALITY, 1_000).is_err());
    }

    #[test]
    fn the_foreground_monitor_is_matched_by_handle_then_name_then_primary() {
        let monitors = vec![
            MonitorRef { id: 0x10001, name: r"\\.\DISPLAY1".into(), primary: true },
            MonitorRef { id: 0x20003, name: r"\\.\DISPLAY2".into(), primary: false },
        ];
        let target = |id: u32, name: &str| MonitorTarget { id, name: name.into() };
        assert_eq!(pick_monitor(&monitors, Some(&target(0x20003, ""))), Some(1));
        assert_eq!(pick_monitor(&monitors, Some(&target(0x99, r"\\.\display2"))), Some(1));
        assert_eq!(pick_monitor(&monitors, Some(&target(0x99, r"\\.\DISPLAY9"))), Some(0), "unknown: the primary");
        assert_eq!(pick_monitor(&monitors, None), Some(0));
        let no_primary = vec![MonitorRef { primary: false, ..monitors[1].clone() }];
        assert_eq!(pick_monitor(&no_primary, None), Some(0));
        assert_eq!(pick_monitor(&[], None), None);
    }

    /// A 5×7 bitmap font for the letters the blur test writes.
    fn glyph(c: char) -> [u8; 7] {
        match c {
            'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
            'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
            'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
            'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
            'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
            'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
            'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
            'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
            'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
            'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
            'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
            'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
            'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
            'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
            _ => [0; 7],
        }
    }

    const WHITE: image::Rgb<u8> = image::Rgb([255, 255, 255]);

    /// Where [`screen_with_text`] writes line `row` of `chars` characters at
    /// `scale`: (x, y, width, height).
    fn line_box(row: usize, chars: usize, scale: u32) -> (u32, u32, u32, u32) {
        (40, 60 + row as u32 * 12 * scale, (chars as u32 * 6 - 1) * scale, 7 * scale)
    }

    /// A white "screen" with lines of black text in the 5×7 font, each dot
    /// `scale` pixels wide: 2 is ordinary UI text (10×14 px glyphs), 4 a
    /// heading.
    fn screen_with_text(lines: &[&str], scale: u32) -> RgbImage {
        let mut frame = RgbImage::from_pixel(960, 540, WHITE);
        for (row, line) in lines.iter().enumerate() {
            let (x0, y0, _, _) = line_box(row, line.chars().count(), scale);
            for (col, c) in line.chars().enumerate() {
                for (gy, bits_row) in glyph(c).iter().enumerate() {
                    for gx in 0..5u32 {
                        if bits_row & (0x10 >> gx) == 0 {
                            continue;
                        }
                        for dy in 0..scale {
                            for dx in 0..scale {
                                let x = x0 + (col as u32 * 6 + gx) * scale + dx;
                                let y = y0 + gy as u32 * scale + dy;
                                frame.put_pixel(x, y, image::Rgb([0, 0, 0]));
                            }
                        }
                    }
                }
            }
        }
        frame
    }

    /// The same screen with each line of text replaced by a flat grey bar
    /// of the same average darkness — everything the blur may leave of text.
    fn screen_with_bars(lines: &[&str], scale: u32) -> RgbImage {
        let text = screen_with_text(lines, scale);
        let mut bars = RgbImage::from_pixel(960, 540, WHITE);
        for (row, line) in lines.iter().enumerate() {
            let (x0, y0, width, height) = line_box(row, line.chars().count(), scale);
            let pixels = || (y0..y0 + height).flat_map(move |y| (x0..x0 + width).map(move |x| (x, y)));
            let sum: u64 = pixels().map(|(x, y)| u64::from(text.get_pixel(x, y)[0])).sum();
            let grey = (sum as f64 / f64::from(width * height)).round() as u8;
            for (x, y) in pixels() {
                bars.put_pixel(x, y, image::Rgb([grey; 3]));
            }
        }
        bars
    }

    fn diff(a: &RgbImage, b: &RgbImage) -> (u8, f64) {
        let mut max = 0u8;
        let mut sum = 0u64;
        for (p, q) in a.as_raw().iter().zip(b.as_raw()) {
            let d = p.abs_diff(*q);
            max = max.max(d);
            sum += u64::from(d);
        }
        (max, sum as f64 / a.as_raw().len() as f64)
    }

    /// The largest difference between horizontally adjacent pixels — the
    /// edges that make up letters.
    fn sharpest_edge(frame: &RgbImage) -> u8 {
        let mut max = 0u8;
        for y in 0..frame.height() {
            for x in 1..frame.width() {
                let (a, b) = (frame.get_pixel(x - 1, y), frame.get_pixel(x, y));
                for channel in 0..3 {
                    max = max.max(a[channel].abs_diff(b[channel]));
                }
            }
        }
        max
    }

    const SECRET: [&str; 3] = ["PASSWORD SECRET", "CONTRACT LEAD", "MEMO TO STAN"];

    #[test]
    fn blurred_text_cannot_be_read() {
        let secret = screen_with_text(&SECRET, 2);
        let bars = screen_with_bars(&SECRET, 2);
        // Before: every letter is plain to see.
        assert!(diff(&secret, &bars).0 >= 128);
        assert_eq!(sharpest_edge(&secret), 255);

        // After: the text is all but identical to flat bars of the same
        // darkness (measured: within 7 of 255 levels). What is left is where
        // text is and how dense it is — the letters, and so what was
        // written, are gone.
        let blurred = blur_unreadable(&secret);
        assert_eq!(blurred.dimensions(), secret.dimensions());
        let (max, _) = diff(&blurred, &blur_unreadable(&bars));
        assert!(max <= 12, "letters still show: {max} levels from plain bars");
        // So two different texts of the same layout look alike...
        let other = blur_unreadable(&screen_with_text(&["DOCUMENT NOTES", "PLANNER STATE", "TODO LATER NOW"], 2));
        assert!(diff(&blurred, &other).0 <= 12, "{:?}", diff(&blurred, &other));
        // ...and no stroke of a letter is left: neighbouring pixels differ
        // by a level or so.
        assert!(sharpest_edge(&blurred) <= 4, "edge of {}", sharpest_edge(&blurred));

        // Large text (a heading) is gone too.
        let heading = ["CONTRACT TERMS"];
        let heading_blurred = blur_unreadable(&screen_with_text(&heading, 4));
        let (max, _) = diff(&heading_blurred, &blur_unreadable(&screen_with_bars(&heading, 4)));
        assert!(max <= 24, "heading still shows: {max} levels from a plain bar");
        assert!(sharpest_edge(&heading_blurred) <= 4);

        // The uploaded capture and its thumbnail carry the blur and say so.
        let capture = encode_capture(secret, true, now()).unwrap();
        assert!(capture.blurred);
        let sent = decode(&capture.jpeg_base64).into_rgb8();
        assert!(diff(&sent, &blurred).0 <= 6, "the JPEG adds no detail back");
        assert!(sharpest_edge(&sent) <= 8, "edge of {}", sharpest_edge(&sent));
        let thumb = decode(capture.thumb_jpg_base64.as_deref().unwrap()).into_rgb8();
        assert!(sharpest_edge(&thumb) <= 12, "edge of {}", sharpest_edge(&thumb));
    }

    #[test]
    fn an_unblurred_capture_keeps_its_text() {
        // The blur is applied only when asked: an ordinary capture stays sharp.
        let capture = encode_capture(screen_with_text(&SECRET, 2), false, now()).unwrap();
        assert!(!capture.blurred);
        assert!(sharpest_edge(&decode(&capture.jpeg_base64).into_rgb8()) >= 128);
    }

    /// Needs a real desktop session: `cargo test -- --ignored`.
    #[test]
    #[ignore = "captures the real screen"]
    fn captures_the_foreground_monitor_with_a_thumbnail() {
        let capture = capture_foreground_monitor(false).unwrap();
        let screen = decode(&capture.jpeg_base64);
        let thumb = decode(capture.thumb_jpg_base64.as_deref().unwrap());
        assert!(screen.width() <= MAX_SCREEN_WIDTH);
        assert_eq!(thumb.width(), THUMB_WIDTH.min(screen.width()));
        let kb = |b64: &str| base64::engine::general_purpose::STANDARD.decode(b64).unwrap().len() / 1024;
        println!(
            "screen {}x{}: {} KB, thumb {}x{} {} KB",
            screen.width(),
            screen.height(),
            kb(&capture.jpeg_base64),
            thumb.width(),
            thumb.height(),
            kb(capture.thumb_jpg_base64.as_deref().unwrap())
        );
        assert!(super::super::active_window::current().is_some());
    }
}
