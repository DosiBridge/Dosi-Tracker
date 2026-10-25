//! Screenshot encoding (ported Windows tests plus the portal PNG path):
//! dimensions, JPEG validity, the server's size caps, and a blur that lowers
//! edge energy until no text can be read. Frames are synthetic; no screen is
//! ever captured here.

use chrono::Utc;
use dosi_tracker_linux::tracking::capture::{
    blur_unreadable, decode_png, encode_capture, encode_frame, encode_jpeg, encode_jpeg_within,
    encode_thumbnail, RawFrame, MAX_SCREEN_BYTES, MAX_SCREEN_WIDTH, MAX_THUMB_BYTES, THUMB_WIDTH,
};
use image::{ImageEncoder, RgbImage};

fn decode(bytes: &[u8]) -> image::DynamicImage {
    // JPEG magic: the server detects the format from it.
    assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF]);
    image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg).unwrap()
}

#[test]
fn thumbnail_is_a_downscaled_jpeg_with_the_same_aspect() {
    let frame = RgbImage::from_pixel(1920, 1080, image::Rgb([40, 90, 200]));
    let jpeg = encode_thumbnail(&frame).unwrap();
    let decoded = decode(&jpeg);
    assert_eq!((decoded.width(), decoded.height()), (480, 270));
    assert!(jpeg.len() <= MAX_THUMB_BYTES);
}

#[test]
fn small_frames_are_not_upscaled() {
    let frame = RgbImage::from_pixel(320, 200, image::Rgb([0, 0, 0]));
    let decoded = decode(&encode_thumbnail(&frame).unwrap());
    assert_eq!((decoded.width(), decoded.height()), (320, 200));
    assert!(encode_thumbnail(&RgbImage::new(0, 0)).is_err());
}

#[test]
fn a_4k_screen_is_sent_as_a_1920_wide_jpeg_with_a_thumbnail() {
    let at = Utc::now();
    let frame = RgbImage::from_fn(3840, 2160, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
    });
    let capture = encode_capture(frame, false, at).unwrap();
    let screen = decode(&capture.jpeg);
    assert_eq!((screen.width(), screen.height()), (1920, 1080));
    let thumb = decode(capture.thumb_jpeg.as_deref().unwrap());
    assert_eq!((thumb.width(), thumb.height()), (480, 270));
    assert_eq!(capture.captured_at, at);
    assert!(!capture.blurred);
    // Far below the server's limit.
    assert!(capture.jpeg.len() < MAX_SCREEN_BYTES / 4);
}

#[test]
fn narrow_screens_keep_their_size() {
    let capture = encode_capture(
        RgbImage::from_pixel(1366, 768, image::Rgb([200, 200, 200])),
        false,
        Utc::now(),
    )
    .unwrap();
    let screen = decode(&capture.jpeg);
    assert_eq!((screen.width(), screen.height()), (1366, 768));
    assert!(screen.width() <= MAX_SCREEN_WIDTH);
    assert!(encode_capture(RgbImage::new(0, 0), false, Utc::now()).is_err());
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
    let full = encode_jpeg(&noise, 80).unwrap().len();
    let fitted = encode_jpeg_within(&noise, 80, full - 1).unwrap();
    assert!(fitted.len() < full, "a lower quality was used");
    assert!(encode_jpeg_within(&noise, 80, 1_000).is_err());
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
    (
        40,
        60 + row as u32 * 12 * scale,
        (chars as u32 * 6 - 1) * scale,
        7 * scale,
    )
}

/// A white "screen" with lines of black text in the 5×7 font, each dot
/// `scale` pixels wide: 2 is ordinary UI text (10×14 px glyphs), 4 a heading.
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

/// The same screen with each line of text replaced by a flat grey bar of the
/// same average darkness — everything the blur may leave of text.
fn screen_with_bars(lines: &[&str], scale: u32) -> RgbImage {
    let text = screen_with_text(lines, scale);
    let mut bars = RgbImage::from_pixel(960, 540, WHITE);
    for (row, line) in lines.iter().enumerate() {
        let (x0, y0, width, height) = line_box(row, line.chars().count(), scale);
        let pixels = || (y0..y0 + height).flat_map(move |y| (x0..x0 + width).map(move |x| (x, y)));
        let sum: u64 = pixels()
            .map(|(x, y)| u64::from(text.get_pixel(x, y)[0]))
            .sum();
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

/// The largest difference between horizontally adjacent pixels — the edges
/// that make up letters.
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

/// Sum of absolute horizontal and vertical neighbour differences: the
/// image's edge energy.
fn edge_energy(frame: &RgbImage) -> u64 {
    let mut energy = 0u64;
    for y in 0..frame.height() {
        for x in 0..frame.width() {
            let p = frame.get_pixel(x, y);
            if x > 0 {
                let q = frame.get_pixel(x - 1, y);
                energy += (0..3).map(|c| u64::from(p[c].abs_diff(q[c]))).sum::<u64>();
            }
            if y > 0 {
                let q = frame.get_pixel(x, y - 1);
                energy += (0..3).map(|c| u64::from(p[c].abs_diff(q[c]))).sum::<u64>();
            }
        }
    }
    energy
}

const SECRET: [&str; 3] = ["PASSWORD SECRET", "CONTRACT LEAD", "MEMO TO STAN"];

#[test]
fn blurred_text_cannot_be_read() {
    let secret = screen_with_text(&SECRET, 2);
    let bars = screen_with_bars(&SECRET, 2);
    // Before: every letter is plain to see.
    assert!(diff(&secret, &bars).0 >= 128);
    assert_eq!(sharpest_edge(&secret), 255);

    // After: the text is all but identical to flat bars of the same darkness.
    let blurred = blur_unreadable(&secret);
    assert_eq!(blurred.dimensions(), secret.dimensions());
    let (max, _) = diff(&blurred, &blur_unreadable(&bars));
    assert!(
        max <= 12,
        "letters still show: {max} levels from plain bars"
    );
    // So two different texts of the same layout look alike...
    let other = blur_unreadable(&screen_with_text(
        &["DOCUMENT NOTES", "PLANNER STATE", "TODO LATER NOW"],
        2,
    ));
    assert!(
        diff(&blurred, &other).0 <= 12,
        "{:?}",
        diff(&blurred, &other)
    );
    // ...and no stroke of a letter is left.
    assert!(
        sharpest_edge(&blurred) <= 4,
        "edge of {}",
        sharpest_edge(&blurred)
    );

    // Large text (a heading) is gone too.
    let heading = ["CONTRACT TERMS"];
    let heading_blurred = blur_unreadable(&screen_with_text(&heading, 4));
    let (max, _) = diff(
        &heading_blurred,
        &blur_unreadable(&screen_with_bars(&heading, 4)),
    );
    assert!(
        max <= 24,
        "heading still shows: {max} levels from a plain bar"
    );
    assert!(sharpest_edge(&heading_blurred) <= 4);

    // The uploaded capture and its thumbnail carry the blur and say so.
    let capture = encode_capture(secret, true, Utc::now()).unwrap();
    assert!(capture.blurred);
    let sent = decode(&capture.jpeg).into_rgb8();
    assert!(diff(&sent, &blurred).0 <= 6, "the JPEG adds no detail back");
    assert!(
        sharpest_edge(&sent) <= 8,
        "edge of {}",
        sharpest_edge(&sent)
    );
    let thumb = decode(capture.thumb_jpeg.as_deref().unwrap()).into_rgb8();
    assert!(
        sharpest_edge(&thumb) <= 12,
        "edge of {}",
        sharpest_edge(&thumb)
    );
}

#[test]
fn blur_lowers_edge_energy_by_orders_of_magnitude() {
    let secret = screen_with_text(&SECRET, 2);
    let before = edge_energy(&secret);
    let after = edge_energy(&blur_unreadable(&secret));
    assert!(before > 0);
    assert!(
        after * 20 < before,
        "edge energy only fell from {before} to {after}"
    );
}

#[test]
fn an_unblurred_capture_keeps_its_text() {
    // The blur is applied only when asked: an ordinary capture stays sharp.
    let capture = encode_capture(screen_with_text(&SECRET, 2), false, Utc::now()).unwrap();
    assert!(!capture.blurred);
    assert!(sharpest_edge(&decode(&capture.jpeg).into_rgb8()) >= 128);
}

#[test]
fn a_portal_png_goes_through_the_same_pipeline() {
    // What the Screenshot portal hands back: a PNG file of the whole screen.
    let screen = RgbImage::from_fn(2560, 1440, |x, y| {
        image::Rgb([(x / 10 % 256) as u8, (y / 10 % 256) as u8, 77])
    });
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            screen.as_raw(),
            screen.width(),
            screen.height(),
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    let decoded = decode_png(&png).unwrap();
    assert_eq!(decoded.dimensions(), (2560, 1440));
    assert_eq!(decoded.get_pixel(15, 25), screen.get_pixel(15, 25));

    let at = Utc::now();
    let capture = encode_frame(
        RawFrame {
            image: decoded,
            captured_at: at,
            monitor: None,
        },
        true,
    )
    .unwrap();
    let sent = decode(&capture.jpeg);
    assert_eq!((sent.width(), sent.height()), (1920, 1080));
    assert!(capture.blurred);
    assert_eq!(capture.captured_at, at);
    let thumb = decode(capture.thumb_jpeg.as_deref().unwrap());
    assert_eq!(thumb.width(), THUMB_WIDTH);
    // RGBA PNGs (what GNOME writes) decode too.
    let rgba = image::RgbaImage::from_pixel(8, 4, image::Rgba([1, 2, 3, 255]));
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(rgba.as_raw(), 8, 4, image::ExtendedColorType::Rgba8)
        .unwrap();
    assert_eq!(
        decode_png(&png).unwrap().get_pixel(0, 0),
        &image::Rgb([1, 2, 3])
    );
}
