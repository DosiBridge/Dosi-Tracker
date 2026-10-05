//! One webcam photo per block, taken on its own thread so a camera that hangs
//! can never stall the worker (and with it Pause and Quit).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use base64::Engine;
use image::imageops::{self, FilterType};
use image::RgbImage;

/// Auto-exposure needs about this long: the first frames of most webcams are
/// dark or green.
const WARMUP: Duration = Duration::from_secs(1);

/// A capture that has not delivered by then is abandoned.
const TIMEOUT: Duration = Duration::from_secs(5);

const JPEG_QUALITY: u8 = 70;

/// Widest photo sent — plenty to see who is at the desk.
const MAX_WIDTH: u32 = 1280;

/// The server's limit on a decoded webcam photo.
pub const MAX_WEBCAM_BYTES: usize = 2 * 1024 * 1024;

/// Set while a capture thread runs, so a camera stuck in a driver call
/// never piles up one blocked thread per block.
static BUSY: AtomicBool = AtomicBool::new(false);

/// Capture a single webcam frame and return it as a base64-encoded JPEG.
///
/// The camera is opened, warmed up, one frame is kept, and it is released
/// immediately so the device is not held between intervals. Gives up after
/// [`TIMEOUT`].
pub fn capture_jpg_base64() -> Result<String> {
    let jpeg = run_with_timeout(TIMEOUT, &BUSY, || encode_photo(grab_frame()?))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(jpeg))
}

fn grab_frame() -> Result<RgbImage> {
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
    use nokhwa::Camera;

    let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
    let mut camera = Camera::new(CameraIndex::Index(0), format)?;
    camera.open_stream()?;
    let frame = (|| {
        let warm_until = Instant::now() + WARMUP;
        let mut frame = camera.frame()?;
        while Instant::now() < warm_until {
            frame = camera.frame()?;
        }
        Ok::<_, anyhow::Error>(frame.decode_image::<RgbFormat>()?)
    })();
    camera.stop_stream().ok();
    frame
}

/// Downscale to at most [`MAX_WIDTH`] and encode as a JPEG within
/// [`MAX_WEBCAM_BYTES`].
fn encode_photo(frame: RgbImage) -> Result<Vec<u8>> {
    use image::ImageEncoder;

    let (width, height) = frame.dimensions();
    if width == 0 || height == 0 {
        return Err(anyhow!("the webcam delivered an empty frame"));
    }
    let frame = if width > MAX_WIDTH {
        let new_height = ((u64::from(height) * u64::from(MAX_WIDTH)) / u64::from(width)).max(1) as u32;
        imageops::resize(&frame, MAX_WIDTH, new_height, FilterType::Triangle)
    } else {
        frame
    };
    for quality in [JPEG_QUALITY, 50, 30] {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality).write_image(
            frame.as_raw(),
            frame.width(),
            frame.height(),
            image::ExtendedColorType::Rgb8,
        )?;
        if jpeg.len() <= MAX_WEBCAM_BYTES {
            return Ok(jpeg);
        }
    }
    Err(anyhow!("the webcam photo is larger than {MAX_WEBCAM_BYTES} bytes"))
}

/// Run `job` on its own thread and wait at most `timeout` for it. A job that
/// overruns is abandoned — its thread ends on its own, or stays blocked in a
/// driver — and while it is still running `busy` refuses a new one.
fn run_with_timeout<T, F>(timeout: Duration, busy: &'static AtomicBool, job: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    /// Clears `busy` however the job ends, panics included.
    struct Release(&'static AtomicBool);
    impl Drop for Release {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }

    if busy.swap(true, Ordering::SeqCst) {
        return Err(anyhow!("a previous webcam capture is still running"));
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new().name("webcam".into()).spawn(move || {
        let _release = Release(busy);
        let _ = tx.send(job());
    });
    if let Err(e) = spawned {
        busy.store(false, Ordering::SeqCst);
        return Err(anyhow!("could not start the webcam capture: {e}"));
    }
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err(anyhow!("the webcam did not deliver a frame within {} s", timeout.as_secs()))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(anyhow!("the webcam capture failed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quick_capture_returns_its_result() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        assert_eq!(run_with_timeout(Duration::from_secs(2), &FLAG, || Ok(7)).unwrap(), 7);
        assert!(run_with_timeout(Duration::from_secs(2), &FLAG, || Err::<(), _>(anyhow!("no camera"))).is_err());
        // Each finished job freed the slot.
        std::thread::sleep(Duration::from_millis(50));
        assert!(!FLAG.load(Ordering::SeqCst));
    }

    #[test]
    fn a_hanging_camera_is_abandoned_and_not_stacked() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let started = Instant::now();
        let result = run_with_timeout(Duration::from_millis(200), &FLAG, || {
            std::thread::sleep(Duration::from_millis(800));
            Ok(())
        });
        assert!(result.unwrap_err().to_string().contains("did not deliver"));
        assert!(started.elapsed() < Duration::from_millis(700), "did not wait for the camera");

        // While it is still stuck, a new capture is refused rather than stacked.
        let refused = run_with_timeout(Duration::from_millis(200), &FLAG, || Ok(()));
        assert!(refused.unwrap_err().to_string().contains("still running"));

        // Once it gives up, the camera can be tried again.
        std::thread::sleep(Duration::from_millis(900));
        assert!(run_with_timeout(Duration::from_secs(2), &FLAG, || Ok(())).is_ok());
    }

    #[test]
    fn a_panicking_capture_is_an_error_and_frees_the_slot() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let result = run_with_timeout(Duration::from_secs(2), &FLAG, || -> Result<()> { panic!("driver exploded") });
        assert!(result.is_err());
        std::thread::sleep(Duration::from_millis(50));
        assert!(!FLAG.load(Ordering::SeqCst));
    }

    #[test]
    fn photos_are_downscaled_jpegs_within_the_limit() {
        let frame = RgbImage::from_fn(1920, 1080, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 90]));
        let jpeg = encode_photo(frame).unwrap();
        assert!(jpeg.len() <= MAX_WEBCAM_BYTES);
        let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1280, 720));

        let small = encode_photo(RgbImage::from_pixel(640, 480, image::Rgb([10, 10, 10]))).unwrap();
        let decoded = image::load_from_memory_with_format(&small, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (640, 480));
        assert!(encode_photo(RgbImage::new(0, 0)).is_err());
    }
}
