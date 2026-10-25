//! X11 screen capture: GetImage of the RandR monitor that shows most of the
//! active window (fallback: the whole root).
//!
//! The active window comes from `_NET_ACTIVE_WINDOW`; its root-relative
//! rectangle from `GetGeometry` plus `TranslateCoordinates`; the monitors from
//! RandR 1.5 `GetMonitors`. The chosen region of the root window is read with
//! `GetImage` (ZPixmap) — supported for the 24/32-bit TrueColor visuals every
//! current X server uses — converted from BGRX to RGB and handed to
//! [`super::capture`] for encoding. Only this one frame is read, on request.

use std::sync::Mutex;

use chrono::Utc;
use image::RgbImage;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, AtomEnum, ConnectionExt as _, ImageFormat, ImageOrder};
use x11rb::rust_connection::RustConnection;

use super::capture::{pick_monitor, MonitorRef, MonitorTarget, RawFrame};
use super::traits::{Availability, CaptureTarget, ScreenSource, TrackingError};
use super::x11::connect_display;

/// A rectangle in root-window coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The area two rectangles share, in pixels.
    pub fn overlap(&self, other: &Rect) -> u64 {
        let left = i64::from(self.x).max(i64::from(other.x));
        let top = i64::from(self.y).max(i64::from(other.y));
        let right = (i64::from(self.x) + i64::from(self.width))
            .min(i64::from(other.x) + i64::from(other.width));
        let bottom = (i64::from(self.y) + i64::from(self.height))
            .min(i64::from(other.y) + i64::from(other.height));
        if right <= left || bottom <= top {
            0
        } else {
            ((right - left) * (bottom - top)) as u64
        }
    }

    /// This rectangle clipped to `bounds`; `None` when nothing is left.
    pub fn clipped_to(&self, bounds: &Rect) -> Option<Rect> {
        let left = self.x.max(bounds.x);
        let top = self.y.max(bounds.y);
        let right = (i64::from(self.x) + i64::from(self.width))
            .min(i64::from(bounds.x) + i64::from(bounds.width));
        let bottom = (i64::from(self.y) + i64::from(self.height))
            .min(i64::from(bounds.y) + i64::from(bounds.height));
        if right <= i64::from(left) || bottom <= i64::from(top) {
            return None;
        }
        Some(Rect::new(
            left,
            top,
            (right - i64::from(left)) as u32,
            (bottom - i64::from(top)) as u32,
        ))
    }
}

/// A RandR monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorGeometry {
    /// Index in the `GetMonitors` reply (the [`MonitorRef::id`]).
    pub index: u32,
    pub name: String,
    pub primary: bool,
    pub rect: Rect,
}

/// The monitor showing most of `window` (ties: the first listed); `None`
/// when there is no window or it is on no monitor.
pub fn monitor_for_window(
    monitors: &[MonitorGeometry],
    window: Option<&Rect>,
) -> Option<MonitorTarget> {
    let window = window?;
    let mut best: Option<(&MonitorGeometry, u64)> = None;
    for monitor in monitors {
        let area = monitor.rect.overlap(window);
        if area > 0 && best.is_none_or(|(_, best_area)| area > best_area) {
            best = Some((monitor, area));
        }
    }
    best.map(|(m, _)| MonitorTarget {
        id: m.index,
        name: m.name.clone(),
    })
}

/// The region to capture: the monitor holding most of the active window
/// (else the primary, else the first monitor), clipped to the root; the
/// whole root without monitors or when asked for the whole screen.
pub fn capture_region(
    root: Rect,
    monitors: &[MonitorGeometry],
    window: Option<&Rect>,
    target: CaptureTarget,
) -> (Rect, Option<MonitorTarget>) {
    if target == CaptureTarget::WholeScreen || monitors.is_empty() {
        return (root, None);
    }
    let refs: Vec<MonitorRef> = monitors
        .iter()
        .map(|m| MonitorRef {
            id: m.index,
            name: m.name.clone(),
            primary: m.primary,
        })
        .collect();
    let wanted = monitor_for_window(monitors, window);
    match pick_monitor(&refs, wanted.as_ref()).map(|i| &monitors[i]) {
        Some(monitor) => match monitor.rect.clipped_to(&root) {
            Some(rect) => (
                rect,
                Some(MonitorTarget {
                    id: monitor.index,
                    name: monitor.name.clone(),
                }),
            ),
            None => (root, None),
        },
        None => (root, None),
    }
}

/// How a ZPixmap image stores its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelLayout {
    pub bits_per_pixel: u8,
    /// Scanlines are padded to a multiple of this many bits.
    pub scanline_pad: u8,
    pub lsb_first: bool,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
}

impl PixelLayout {
    /// The usual 24/32-bit TrueColor layout of a little-endian server
    /// (BGRX bytes).
    pub const BGRX: PixelLayout = PixelLayout {
        bits_per_pixel: 32,
        scanline_pad: 32,
        lsb_first: true,
        red_mask: 0x00ff_0000,
        green_mask: 0x0000_ff00,
        blue_mask: 0x0000_00ff,
    };
}

fn channel(pixel: u32, mask: u32) -> u8 {
    ((pixel & mask) >> mask.trailing_zeros()) as u8
}

/// Convert ZPixmap `data` of a `width` x `height` image to RGB. Only 32 bits
/// per pixel with 8-bit channels are supported (24- and 32-bit depths).
pub fn zpixmap_to_rgb(
    data: &[u8],
    width: u32,
    height: u32,
    layout: PixelLayout,
) -> Result<RgbImage, TrackingError> {
    let eight_bit = |mask: u32| mask != 0 && (mask >> mask.trailing_zeros()) == 0xff;
    if layout.bits_per_pixel != 32
        || !eight_bit(layout.red_mask)
        || !eight_bit(layout.green_mask)
        || !eight_bit(layout.blue_mask)
    {
        return Err(TrackingError::UnsupportedSession(format!(
            "screen capture supports 24/32-bit TrueColor visuals only ({} bits per pixel)",
            layout.bits_per_pixel
        )));
    }
    let pad = usize::from(layout.scanline_pad.max(8)) / 8;
    let row_bytes = (width as usize * 4).div_ceil(pad) * pad;
    let needed = row_bytes
        .checked_mul(height as usize)
        .ok_or_else(|| TrackingError::ProtocolError("image too large".into()))?;
    if data.len() < needed {
        return Err(TrackingError::ProtocolError(format!(
            "GetImage returned {} bytes, {} expected",
            data.len(),
            needed
        )));
    }
    let mut out = RgbImage::new(width, height);
    for (y, row) in data
        .chunks_exact(row_bytes)
        .take(height as usize)
        .enumerate()
    {
        for (x, px) in row.chunks_exact(4).take(width as usize).enumerate() {
            let bytes = [px[0], px[1], px[2], px[3]];
            let pixel = if layout.lsb_first {
                u32::from_le_bytes(bytes)
            } else {
                u32::from_be_bytes(bytes)
            };
            out.put_pixel(
                x as u32,
                y as u32,
                image::Rgb([
                    channel(pixel, layout.red_mask),
                    channel(pixel, layout.green_mask),
                    channel(pixel, layout.blue_mask),
                ]),
            );
        }
    }
    Ok(out)
}

struct Session {
    conn: RustConnection,
    screen: usize,
    net_active_window: xproto::Atom,
    randr: bool,
}

/// X11 screen source for one explicit display, connected lazily.
pub struct X11ScreenSource {
    display: String,
    session: Mutex<Option<Session>>,
    last_failure: Mutex<Option<TrackingError>>,
}

fn proto(context: &str, e: impl std::fmt::Display) -> TrackingError {
    TrackingError::ProtocolError(format!("{context}: {e}"))
}

fn reply_error(context: &str, e: x11rb::errors::ReplyError) -> TrackingError {
    match e {
        x11rb::errors::ReplyError::ConnectionError(e) => {
            TrackingError::DisplayUnavailable(format!("X11 connection lost ({context}): {e}"))
        }
        other => proto(context, other),
    }
}

impl X11ScreenSource {
    /// A source for the explicit X11 `display`; nothing is opened yet.
    pub fn with_display(display: impl Into<String>) -> Self {
        Self {
            display: display.into(),
            session: Mutex::new(None),
            last_failure: Mutex::new(None),
        }
    }

    fn connect(&self) -> Result<Session, TrackingError> {
        let (conn, screen) = connect_display(&self.display)?;
        let net_active_window = conn
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")
            .map_err(|e| proto("InternAtom", e))?
            .reply()
            .map_err(|e| reply_error("InternAtom", e))?
            .atom;
        let randr = match conn
            .extension_information(randr::X11_EXTENSION_NAME)
            .map_err(|e| proto("QueryExtension", e))?
        {
            Some(_) => conn
                .randr_query_version(1, 5)
                .map_err(|e| proto("RRQueryVersion", e))?
                .reply()
                .map(|v| (v.major_version, v.minor_version) >= (1, 5))
                .unwrap_or(false),
            None => false,
        };
        Ok(Session {
            conn,
            screen,
            net_active_window,
            randr,
        })
    }

    fn capture_with(session: &Session, target: CaptureTarget) -> Result<RawFrame, TrackingError> {
        let conn = &session.conn;
        let setup = conn.setup();
        let screen = setup
            .roots
            .get(session.screen)
            .ok_or_else(|| TrackingError::DisplayUnavailable("no X11 screen".into()))?;
        let root = Rect::new(
            0,
            0,
            u32::from(screen.width_in_pixels),
            u32::from(screen.height_in_pixels),
        );

        let monitors = if session.randr && target == CaptureTarget::ActiveMonitor {
            Self::monitors(conn, screen.root)?
        } else {
            Vec::new()
        };
        let window = if monitors.is_empty() {
            None
        } else {
            Self::active_window_rect(conn, screen.root, session.net_active_window)?
        };
        let (region, monitor) = capture_region(root, &monitors, window.as_ref(), target);

        let captured_at = Utc::now();
        let image = conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                screen.root,
                region.x as i16,
                region.y as i16,
                region.width as u16,
                region.height as u16,
                !0,
            )
            .map_err(|e| proto("GetImage", e))?
            .reply()
            .map_err(|e| reply_error("GetImage", e))?;

        let format = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == image.depth)
            .ok_or_else(|| proto("GetImage", "no pixmap format for the image depth"))?;
        let visual = screen
            .allowed_depths
            .iter()
            .flat_map(|d| d.visuals.iter())
            .find(|v| v.visual_id == image.visual)
            .ok_or_else(|| proto("GetImage", "unknown visual"))?;
        let layout = PixelLayout {
            bits_per_pixel: format.bits_per_pixel,
            scanline_pad: format.scanline_pad,
            lsb_first: setup.image_byte_order == ImageOrder::LSB_FIRST,
            red_mask: visual.red_mask,
            green_mask: visual.green_mask,
            blue_mask: visual.blue_mask,
        };
        let image = zpixmap_to_rgb(&image.data, region.width, region.height, layout)?;
        Ok(RawFrame {
            image,
            captured_at,
            monitor,
        })
    }

    fn monitors(
        conn: &RustConnection,
        root: xproto::Window,
    ) -> Result<Vec<MonitorGeometry>, TrackingError> {
        let reply = conn
            .randr_get_monitors(root, true)
            .map_err(|e| proto("RRGetMonitors", e))?
            .reply()
            .map_err(|e| reply_error("RRGetMonitors", e))?;
        let mut monitors = Vec::with_capacity(reply.monitors.len());
        for (index, m) in reply.monitors.iter().enumerate() {
            let name = conn
                .get_atom_name(m.name)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                .unwrap_or_default();
            monitors.push(MonitorGeometry {
                index: index as u32,
                name,
                primary: m.primary,
                rect: Rect::new(
                    i32::from(m.x),
                    i32::from(m.y),
                    u32::from(m.width),
                    u32::from(m.height),
                ),
            });
        }
        Ok(monitors)
    }

    /// The active window's rectangle in root coordinates; `None` when no
    /// window is active (or it vanished).
    fn active_window_rect(
        conn: &RustConnection,
        root: xproto::Window,
        net_active_window: xproto::Atom,
    ) -> Result<Option<Rect>, TrackingError> {
        let reply = conn
            .get_property(false, root, net_active_window, AtomEnum::WINDOW, 0, 1)
            .map_err(|e| proto("GetProperty", e))?
            .reply()
            .map_err(|e| reply_error("GetProperty", e))?;
        let Some(window) = reply.value32().and_then(|mut v| v.next()) else {
            return Ok(None);
        };
        if window == 0 || window == root {
            return Ok(None);
        }
        let geometry = match conn
            .get_geometry(window)
            .map_err(|e| proto("GetGeometry", e))?
            .reply()
        {
            Ok(g) => g,
            Err(x11rb::errors::ReplyError::X11Error(_)) => return Ok(None),
            Err(e) => return Err(reply_error("GetGeometry", e)),
        };
        let origin = match conn
            .translate_coordinates(window, root, 0, 0)
            .map_err(|e| proto("TranslateCoordinates", e))?
            .reply()
        {
            Ok(t) => t,
            Err(x11rb::errors::ReplyError::X11Error(_)) => return Ok(None),
            Err(e) => return Err(reply_error("TranslateCoordinates", e)),
        };
        Ok(Some(Rect::new(
            i32::from(origin.dst_x),
            i32::from(origin.dst_y),
            u32::from(geometry.width),
            u32::from(geometry.height),
        )))
    }
}

impl ScreenSource for X11ScreenSource {
    fn capture(&self, target: CaptureTarget) -> Result<RawFrame, TrackingError> {
        let mut guard = self.session.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            match self.connect() {
                Ok(session) => *guard = Some(session),
                Err(e) => {
                    *self.last_failure.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.clone());
                    return Err(e);
                }
            }
        }
        let result = Self::capture_with(guard.as_ref().expect("connected above"), target);
        if let Err(TrackingError::DisplayUnavailable(_)) = &result {
            *guard = None;
        }
        *self.last_failure.lock().unwrap_or_else(|p| p.into_inner()) =
            result.as_ref().err().cloned();
        result
    }

    fn availability(&self) -> Availability {
        if self.display.trim().is_empty() {
            return Availability::Unsupported("no X11 display".into());
        }
        match &*self.last_failure.lock().unwrap_or_else(|p| p.into_inner()) {
            None => Availability::Available,
            Some(TrackingError::UnsupportedSession(reason)) => {
                Availability::Unsupported(reason.clone())
            }
            Some(other) => Availability::Unavailable(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(index: u32, name: &str, primary: bool, rect: Rect) -> MonitorGeometry {
        MonitorGeometry {
            index,
            name: name.into(),
            primary,
            rect,
        }
    }

    fn dual() -> Vec<MonitorGeometry> {
        vec![
            monitor(0, "LEFT", true, Rect::new(0, 0, 1920, 1080)),
            monitor(1, "RIGHT", false, Rect::new(1920, 0, 2560, 1440)),
        ]
    }

    const ROOT: Rect = Rect {
        x: 0,
        y: 0,
        width: 4480,
        height: 1440,
    };

    #[test]
    fn overlap_and_clipping() {
        let a = Rect::new(0, 0, 100, 100);
        assert_eq!(a.overlap(&Rect::new(50, 50, 100, 100)), 2500);
        assert_eq!(a.overlap(&Rect::new(100, 0, 10, 10)), 0);
        assert_eq!(a.overlap(&Rect::new(-50, -50, 60, 60)), 100);
        assert_eq!(
            Rect::new(-10, -10, 50, 50).clipped_to(&a),
            Some(Rect::new(0, 0, 40, 40))
        );
        assert_eq!(Rect::new(200, 0, 5, 5).clipped_to(&a), None);
    }

    #[test]
    fn the_monitor_holding_most_of_the_window_is_captured() {
        // Mostly on the right monitor, a little on the left.
        let window = Rect::new(1800, 100, 800, 600);
        let (region, target) =
            capture_region(ROOT, &dual(), Some(&window), CaptureTarget::ActiveMonitor);
        assert_eq!(region, Rect::new(1920, 0, 2560, 1440));
        assert_eq!(target.unwrap().name, "RIGHT");

        // No window, or a window off every monitor: the primary.
        for window in [None, Some(Rect::new(9000, 9000, 10, 10))] {
            let (region, target) =
                capture_region(ROOT, &dual(), window.as_ref(), CaptureTarget::ActiveMonitor);
            assert_eq!(region, Rect::new(0, 0, 1920, 1080));
            assert_eq!(target.unwrap().name, "LEFT");
        }

        // Asked for the whole screen, or no monitors known: the root.
        assert_eq!(
            capture_region(ROOT, &dual(), Some(&window), CaptureTarget::WholeScreen),
            (ROOT, None)
        );
        assert_eq!(
            capture_region(ROOT, &[], Some(&window), CaptureTarget::ActiveMonitor),
            (ROOT, None)
        );
    }

    #[test]
    fn bgrx_pixels_become_rgb() {
        // 2x1 image: pure red, then (10, 20, 30); X byte garbage.
        let data = [0x00, 0x00, 0xff, 0x77, 30, 20, 10, 0x99];
        let image = zpixmap_to_rgb(&data, 2, 1, PixelLayout::BGRX).unwrap();
        assert_eq!(image.get_pixel(0, 0), &image::Rgb([255, 0, 0]));
        assert_eq!(image.get_pixel(1, 0), &image::Rgb([10, 20, 30]));

        // A big-endian server sends XRGB.
        let msb = PixelLayout {
            lsb_first: false,
            ..PixelLayout::BGRX
        };
        let image = zpixmap_to_rgb(&[0x00, 10, 20, 30], 1, 1, msb).unwrap();
        assert_eq!(image.get_pixel(0, 0), &image::Rgb([10, 20, 30]));
    }

    #[test]
    fn unsupported_or_short_images_are_refused() {
        let sixteen = PixelLayout {
            bits_per_pixel: 16,
            red_mask: 0xf800,
            green_mask: 0x07e0,
            blue_mask: 0x001f,
            ..PixelLayout::BGRX
        };
        assert!(matches!(
            zpixmap_to_rgb(&[0; 8], 2, 2, sixteen),
            Err(TrackingError::UnsupportedSession(_))
        ));
        assert!(matches!(
            zpixmap_to_rgb(&[0; 7], 2, 1, PixelLayout::BGRX),
            Err(TrackingError::ProtocolError(_))
        ));
    }

    #[test]
    fn a_source_without_a_display_is_unsupported_and_opens_nothing() {
        let source = X11ScreenSource::with_display("");
        assert!(matches!(
            source.availability(),
            Availability::Unsupported(_)
        ));
        assert!(matches!(
            source.capture(CaptureTarget::ActiveMonitor),
            Err(TrackingError::DisplayUnavailable(_))
        ));
    }
}
