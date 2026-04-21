//! Media probe — extract MIME, dimensions, EXIF, GPS, device info from a file.
//!
//! Phase 1 covers JPEG / PNG / GIF / WebP / BMP / TIFF / HEIC/HEIF and
//! common video containers. RAW is *flagged* (`is_raw=true`) with dimensions
//! when `rawloader` can open it, otherwise MIME-only per plan §370.

// The probe module makes no direct FFI calls — it goes through `libheif-rs`
// (safe wrapper) and `ffmpeg-next` (safe wrapper). Unsafe isn't needed here;
// the workspace-level deny(unsafe_code) suffices.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Structured result of probing a media file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MediaProbe {
    pub mime: String,
    pub bytes: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub taken_at_utc: Option<DateTime<Utc>>,
    /// (lat, lon, alt_m). Alt is optional.
    pub gps: Option<(f64, f64, Option<f64>)>,
    pub device: Option<String>,
    pub lens: Option<String>,
    pub is_raw: bool,
    pub is_screenshot: bool,
    pub is_live: bool,
    pub is_motion: bool,
    pub exif_all_json: serde_json::Value,
}

impl MediaProbe {
    /// Epoch-day, if the probe produced a timestamp. Used for the plaintext
    /// `asset.taken_at_utc_day` column.
    pub fn taken_at_utc_day(&self) -> Option<i64> {
        self.taken_at_utc.map(|t| t.timestamp() / 86_400)
    }
}

/// Probe the file at `path`. Never panics on malformed input — a file whose
/// EXIF / container header can't be read still yields a `MediaProbe` with
/// MIME + `bytes` populated.
pub fn probe_path(path: &Path) -> Result<MediaProbe> {
    let meta = std::fs::metadata(path)?;
    let bytes = meta.len();
    let (mime, ext) = sniff_mime(path)?;
    let mut probe = MediaProbe {
        mime: mime.clone(),
        bytes,
        ..Default::default()
    };

    probe.is_screenshot = looks_like_screenshot(path, &mime);

    // Major-type dispatch.
    if mime.starts_with("image/") {
        if mime == "image/heic" || mime == "image/heif" {
            let _ = probe_heic(path, &mut probe);
        } else {
            let _ = probe_basic_image(path, &mut probe);
        }
        // EXIF works for JPEG/TIFF/HEIC; harmless to attempt on others.
        let _ = probe_exif(path, &mut probe);
    } else if mime.starts_with("video/") {
        let _ = probe_video(path, &mut probe);
        probe.exif_all_json = serde_json::json!({"mime": mime.clone()});
    } else if is_raw_ext(ext.as_deref()) {
        probe.is_raw = true;
        let _ = probe_raw(path, &mut probe);
    }

    if probe.exif_all_json.is_null() {
        probe.exif_all_json = serde_json::json!({});
    }
    Ok(probe)
}

// --------- MIME sniffing ------------------------------------------------------

fn sniff_mime(path: &Path) -> Result<(String, Option<String>)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());

    // `infer` sniffs content; fall back to extension for things it doesn't
    // recognise (HEIC from some encoders, exotic RAW).
    let mut f = File::open(path)?;
    let mut head = [0u8; 512];
    let n = f.read(&mut head)?;
    if let Some(kind) = infer::get(&head[..n]) {
        return Ok((kind.mime_type().to_string(), ext));
    }

    let by_ext = match ext.as_deref() {
        Some("heic") | Some("heif") => "image/heic",
        Some("cr2") | Some("cr3") | Some("nef") | Some("arw") | Some("dng") | Some("raf")
        | Some("orf") | Some("rw2") => "application/octet-stream",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("tif") | Some("tiff") => "image/tiff",
        Some("mp4") | Some("m4v") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    };
    Ok((by_ext.to_string(), ext))
}

fn is_raw_ext(ext: Option<&str>) -> bool {
    matches!(
        ext,
        Some("cr2" | "cr3" | "nef" | "arw" | "dng" | "raf" | "orf" | "rw2")
    )
}

// --------- Image probes -------------------------------------------------------

fn probe_basic_image(path: &Path, probe: &mut MediaProbe) -> Result<()> {
    // `image::io::Reader` already does format detection for the formats we
    // care about here (JPEG / PNG / GIF / WebP / BMP / TIFF).
    match image::ImageReader::open(path) {
        Ok(r) => match r.with_guessed_format() {
            Ok(r) => {
                if let Ok(dim) = r.into_dimensions() {
                    probe.width = Some(dim.0);
                    probe.height = Some(dim.1);
                }
            }
            Err(e) => tracing::debug!(?e, "image reader format guess failed"),
        },
        Err(e) => tracing::debug!(?e, "image reader open failed"),
    }
    Ok(())
}

fn probe_heic(path: &Path, probe: &mut MediaProbe) -> Result<()> {
    use libheif_rs::{HeifContext, LibHeif};
    let ctx = match HeifContext::read_from_file(path.to_str().ok_or(Error::Crypto)?) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(?e, "heif open failed");
            return Ok(());
        }
    };
    // Force libheif init so the primary-image handle lookup works.
    let _ = LibHeif::new();
    if let Ok(handle) = ctx.primary_image_handle() {
        probe.width = Some(handle.width());
        probe.height = Some(handle.height());
    }
    Ok(())
}

fn probe_raw(path: &Path, probe: &mut MediaProbe) -> Result<()> {
    // rawloader can decode most common RAWs. Dimensions are always available
    // via `decode_file`; deeper EXIF pull is deferred to Phase 2.
    match rawloader::decode_file(path) {
        Ok(img) => {
            probe.width = Some(img.width as u32);
            probe.height = Some(img.height as u32);
            probe.device = Some(format!("{} {}", img.make, img.model).trim().to_string());
        }
        Err(e) => tracing::debug!(
            ?e,
            "rawloader decode failed — falling back to flag-only RAW"
        ),
    }
    Ok(())
}

// --------- EXIF (JPEG / TIFF / HEIC) ------------------------------------------

fn probe_exif(path: &Path, probe: &mut MediaProbe) -> Result<()> {
    // `kamadak-exif` is robust for JPEG; give it first dibs.
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(()),
    };
    let mut bufreader = std::io::BufReader::new(file);
    let exif = match exif::Reader::new().read_from_container(&mut bufreader) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    let mut j = serde_json::Map::new();

    for f in exif.fields() {
        let key = format!("{}::{}", f.ifd_num, f.tag);
        j.insert(
            key,
            serde_json::Value::String(f.display_value().to_string()),
        );
    }

    let get = |t: exif::Tag, ifd: exif::In| {
        exif.get_field(t, ifd)
            .map(|f| f.display_value().with_unit(&exif).to_string())
    };

    probe.device = get(exif::Tag::Model, exif::In::PRIMARY)
        .map(|m| m.trim_matches('"').to_string())
        .filter(|s| !s.is_empty());
    probe.lens = get(exif::Tag::LensModel, exif::In::PRIMARY)
        .map(|m| m.trim_matches('"').to_string())
        .filter(|s| !s.is_empty());

    // Width / height from EXIF as a fallback if `image` couldn't read them.
    if probe.width.is_none() {
        if let Some(f) = exif.get_field(exif::Tag::PixelXDimension, exif::In::PRIMARY) {
            if let Some(v) = f.value.get_uint(0) {
                probe.width = Some(v);
            }
        }
    }
    if probe.height.is_none() {
        if let Some(f) = exif.get_field(exif::Tag::PixelYDimension, exif::In::PRIMARY) {
            if let Some(v) = f.value.get_uint(0) {
                probe.height = Some(v);
            }
        }
    }

    // Timestamp: prefer DateTimeOriginal, fall back to DateTime.
    for tag in [exif::Tag::DateTimeOriginal, exif::Tag::DateTime] {
        if let Some(f) = exif.get_field(tag, exif::In::PRIMARY) {
            let s = f.display_value().to_string();
            if let Ok(dt) = NaiveDateTime::parse_from_str(s.trim_matches('"'), "%Y-%m-%d %H:%M:%S")
            {
                probe.taken_at_utc = Some(dt.and_utc());
                break;
            }
            if let Ok(dt) = NaiveDateTime::parse_from_str(s.trim_matches('"'), "%Y:%m:%d %H:%M:%S")
            {
                probe.taken_at_utc = Some(dt.and_utc());
                break;
            }
        }
    }

    // GPS — EXIF stores rationals for degrees/minutes/seconds.
    if let (Some(lat), Some(latref), Some(lon), Some(lonref)) = (
        exif.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY),
        exif.get_field(exif::Tag::GPSLatitudeRef, exif::In::PRIMARY),
        exif.get_field(exif::Tag::GPSLongitude, exif::In::PRIMARY),
        exif.get_field(exif::Tag::GPSLongitudeRef, exif::In::PRIMARY),
    ) {
        let lat = dms_to_decimal(&lat.value).map(|d| {
            if latref.display_value().to_string().contains('S') {
                -d
            } else {
                d
            }
        });
        let lon = dms_to_decimal(&lon.value).map(|d| {
            if lonref.display_value().to_string().contains('W') {
                -d
            } else {
                d
            }
        });
        let alt = exif
            .get_field(exif::Tag::GPSAltitude, exif::In::PRIMARY)
            .and_then(|f| rational_to_f64(&f.value));
        if let (Some(lat), Some(lon)) = (lat, lon) {
            probe.gps = Some((lat, lon, alt));
        }
    }

    probe.exif_all_json = serde_json::Value::Object(j);
    Ok(())
}

fn dms_to_decimal(v: &exif::Value) -> Option<f64> {
    match v {
        exif::Value::Rational(r) if r.len() == 3 => {
            let d = r[0].to_f64();
            let m = r[1].to_f64();
            let s = r[2].to_f64();
            Some(d + m / 60.0 + s / 3600.0)
        }
        _ => None,
    }
}

fn rational_to_f64(v: &exif::Value) -> Option<f64> {
    match v {
        exif::Value::Rational(r) if !r.is_empty() => Some(r[0].to_f64()),
        _ => None,
    }
}

// --------- Video probes -------------------------------------------------------

fn probe_video(path: &Path, probe: &mut MediaProbe) -> Result<()> {
    // We guard against ffmpeg-next init happening on every call; it's cheap
    // but the library asks us to call it exactly once.
    static FFMPEG_INIT: std::sync::Once = std::sync::Once::new();
    FFMPEG_INIT.call_once(|| {
        // Silence ffmpeg's default INFO-level chatter.
        ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Error);
        let _ = ffmpeg_next::init();
    });

    let input = match ffmpeg_next::format::input(path) {
        Ok(i) => i,
        Err(e) => {
            tracing::debug!(?e, "ffmpeg input open failed");
            return Ok(());
        }
    };
    let dur = input.duration(); // in AV_TIME_BASE units
    if dur > 0 {
        let ms = dur as f64 * 1000.0 / f64::from(ffmpeg_next::ffi::AV_TIME_BASE);
        probe.duration_ms = Some(ms as u64);
    }
    if let Some(stream) = input.streams().best(ffmpeg_next::media::Type::Video) {
        if let Ok(decoder) =
            ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())
        {
            if let Ok(video) = decoder.decoder().video() {
                let w = video.width();
                let h = video.height();
                if w > 0 {
                    probe.width = Some(w);
                }
                if h > 0 {
                    probe.height = Some(h);
                }
            }
        }
    }
    Ok(())
}

// --------- Screenshot heuristics ---------------------------------------------

fn looks_like_screenshot(path: &Path, mime: &str) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // iOS screenshots: IMG_####.PNG with UUID in MakerNote (we just check name)
    // Android: Screenshot_YYYYMMDD-HHMMSS or Screenshot_N.png
    // macOS:  "Screen Shot YYYY-MM-DD..."
    if name.starts_with("screenshot") || name.starts_with("screen shot") {
        return true;
    }
    // Parent dir named 'Screenshots' (Android).
    if let Some(parent) = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
    {
        if parent.eq_ignore_ascii_case("Screenshots") {
            return true;
        }
    }
    // iOS screenshots tend to be PNG not HEIC.
    if mime == "image/png" && name.starts_with("img_") {
        // Not a reliable signal on its own, so stay conservative.
    }
    false
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use tempfile::TempDir;

    fn tiny_jpeg(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(8, 6, |x, y| Rgb([(x * 32) as u8, (y * 48) as u8, 128]));
        img.save_with_format(&p, image::ImageFormat::Jpeg).unwrap();
        p
    }

    fn tiny_png(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(10, 4, |_, _| Rgb([1, 2, 3]));
        img.save_with_format(&p, image::ImageFormat::Png).unwrap();
        p
    }

    #[test]
    fn probe_jpeg_returns_dimensions_and_mime() {
        let dir = TempDir::new().unwrap();
        let path = tiny_jpeg(dir.path(), "a.jpg");
        let p = probe_path(&path).unwrap();
        assert_eq!(p.mime, "image/jpeg");
        assert_eq!(p.width, Some(8));
        assert_eq!(p.height, Some(6));
        assert!(p.bytes > 0);
        assert!(!p.is_raw);
        assert!(!p.is_screenshot);
    }

    #[test]
    fn probe_png_returns_dimensions() {
        let dir = TempDir::new().unwrap();
        let path = tiny_png(dir.path(), "a.png");
        let p = probe_path(&path).unwrap();
        assert_eq!(p.mime, "image/png");
        assert_eq!(p.width, Some(10));
        assert_eq!(p.height, Some(4));
    }

    #[test]
    fn screenshot_heuristic_filename() {
        let dir = TempDir::new().unwrap();
        // Name-based detection.
        let path = tiny_png(dir.path(), "Screenshot_20260101.png");
        let p = probe_path(&path).unwrap();
        assert!(p.is_screenshot, "filename-based screenshot not detected");
    }

    #[test]
    fn screenshot_heuristic_parent_dir() {
        let dir = TempDir::new().unwrap();
        let screens = dir.path().join("Screenshots");
        std::fs::create_dir_all(&screens).unwrap();
        let path = tiny_png(&screens, "random.png");
        let p = probe_path(&path).unwrap();
        assert!(p.is_screenshot);
    }

    #[test]
    fn raw_extension_is_flagged_even_without_exif() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("IMG_0001.CR2");
        std::fs::write(&path, b"not actually a CR2 but we flag by extension").unwrap();
        let p = probe_path(&path).unwrap();
        assert!(p.is_raw);
    }

    #[test]
    fn taken_at_utc_day_is_days_since_epoch() {
        let mut probe = MediaProbe::default();
        probe.taken_at_utc = Some("2024-06-01T12:30:00Z".parse().unwrap());
        // 2024-06-01 is day 19875.
        assert_eq!(probe.taken_at_utc_day(), Some(19875));
    }
}
