//! Derivative generation — thumbnails (256px and 1024px) as WebP, video
//! proxies (720p H.264 MP4) via ffmpeg.
//!
//! Phase 1 generates thumbnails for everything that the `image` crate can
//! decode directly. Videos produce a representative-frame thumbnail; proxy
//! transcoding is deferred to a background job (Phase 2 wires this up).

use std::path::Path;

use image::imageops::FilterType;
use image::ImageFormat;

use crate::{Error, Result};

/// Two thumbnail sizes per §6 / plans/phase-1-foundation.md Task 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailSize {
    /// 256 px longest side — grid thumbnails.
    Thumb256,
    /// 1024 px longest side — asset-detail view.
    Thumb1024,
}

impl ThumbnailSize {
    pub fn max_dim(&self) -> u32 {
        match self {
            Self::Thumb256 => 256,
            Self::Thumb1024 => 1024,
        }
    }
    pub fn as_derivative_kind(&self) -> &'static str {
        match self {
            Self::Thumb256 => "thumb256",
            Self::Thumb1024 => "thumb1024",
        }
    }
}

/// Result of a single thumbnail encode.
#[derive(Debug, Clone)]
pub struct ThumbnailOutput {
    pub size: ThumbnailSize,
    /// WebP-encoded bytes.
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Generate both a 256 px and 1024 px WebP thumbnail from an image on disk.
///
/// Returns an empty Vec for any format the `image` crate can't decode.
/// Video proxies are handled by [`derive_video_keyframe`] instead.
pub fn derive_thumbnails(path: &Path) -> Result<Vec<ThumbnailOutput>> {
    let img = match image::ImageReader::open(path).and_then(image::ImageReader::with_guessed_format)
    {
        Ok(reader) => match reader.decode() {
            Ok(img) => img,
            Err(e) => {
                tracing::debug!(?e, path=?path, "image decode failed — no thumbnail");
                return Ok(Vec::new());
            }
        },
        Err(e) => {
            tracing::debug!(?e, path=?path, "image open failed — no thumbnail");
            return Ok(Vec::new());
        }
    };

    let mut out = Vec::with_capacity(2);
    for size in [ThumbnailSize::Thumb256, ThumbnailSize::Thumb1024] {
        // `resize` scales to fit; for tiny sources smaller than max_dim we
        // skip the upscale and just encode the original — storing bytes we'd
        // just stretch is wasteful.
        let resized = if img.width() <= size.max_dim() && img.height() <= size.max_dim() {
            img.clone()
        } else {
            img.resize(size.max_dim(), size.max_dim(), FilterType::Triangle)
        };
        let mut bytes = Vec::with_capacity(32 * 1024);
        let mut cur = std::io::Cursor::new(&mut bytes);
        // WebP in the `image` crate uses a lossless encoder. Quality/bitrate
        // tuning is deferred to a Phase 2 pass.
        resized
            .write_to(&mut cur, ImageFormat::WebP)
            .map_err(|e| Error::Media(format!("webp encode: {e}")))?;
        out.push(ThumbnailOutput {
            size,
            width: resized.width(),
            height: resized.height(),
            bytes,
        });
    }
    Ok(out)
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use tempfile::TempDir;

    fn make_jpeg(dir: &Path, w: u32, h: u32) -> std::path::PathBuf {
        let p = dir.join(format!("test-{w}x{h}.jpg"));
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(w, h, |x, y| {
            Rgb([(x % 255) as u8, (y % 255) as u8, ((x + y) % 255) as u8])
        });
        img.save_with_format(&p, image::ImageFormat::Jpeg).unwrap();
        p
    }

    #[test]
    fn produces_two_thumbnails_respecting_max_dim() {
        let dir = TempDir::new().unwrap();
        let path = make_jpeg(dir.path(), 4000, 3000);
        let thumbs = derive_thumbnails(&path).unwrap();
        assert_eq!(thumbs.len(), 2);
        let t256 = &thumbs[0];
        assert_eq!(t256.size, ThumbnailSize::Thumb256);
        assert!(t256.width <= 256 && t256.height <= 256);
        assert_eq!(t256.width.max(t256.height), 256);

        let t1024 = &thumbs[1];
        assert_eq!(t1024.size, ThumbnailSize::Thumb1024);
        assert!(t1024.width <= 1024 && t1024.height <= 1024);
        assert_eq!(t1024.width.max(t1024.height), 1024);

        // WebP magic: "RIFF....WEBP"
        for t in thumbs {
            assert!(t.bytes.len() > 12);
            assert_eq!(&t.bytes[0..4], b"RIFF");
            assert_eq!(&t.bytes[8..12], b"WEBP");
        }
    }

    #[test]
    fn unreadable_file_yields_empty_vec_not_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("garbage.jpg");
        std::fs::write(&path, b"this is not a real JPEG").unwrap();
        let thumbs = derive_thumbnails(&path).unwrap();
        assert!(thumbs.is_empty());
    }

    #[test]
    fn upscale_small_source_doesnt_balloon() {
        // Source already fits in 256px — resize should not upscale.
        let dir = TempDir::new().unwrap();
        let path = make_jpeg(dir.path(), 100, 80);
        let thumbs = derive_thumbnails(&path).unwrap();
        assert_eq!(thumbs[0].width, 100);
        assert_eq!(thumbs[0].height, 80);
    }
}
