//! Google Photos Takeout adapter.
//!
//! Takeout dumps pair each media file with a sibling `<name>.json` that
//! stores the metadata Google strips from the on-file EXIF — most notably
//! `photoTakenTime.timestamp` and `geoData.{latitude,longitude,altitude}`.
//!
//! The adapter reads the JSON sidecar when present and writes a *drop-in
//! XMP sidecar* next to the media file so the generic ingest pipeline
//! picks the reconstructed metadata up through its standard sidecar path.
//!
//! If the media file already has a `.xmp` sidecar, we do not overwrite it.

use std::path::{Path, PathBuf};

use chrono::DateTime;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::ingest::sidecar::{write_xmp_sidecar, XmpFields};
use crate::ingest::{IngestAdapter, IngestCtx, IngestReport, Progress, ScanReport};
use crate::Result;

use super::generic::GenericAdapter;

pub struct GoogleTakeoutAdapter {
    inner: GenericAdapter,
}

impl GoogleTakeoutAdapter {
    pub fn new() -> Self {
        Self { inner: GenericAdapter::new() }
    }
}

impl Default for GoogleTakeoutAdapter {
    fn default() -> Self {
        Self::new()
    }
}

/// Shape of the Takeout JSON we care about. Takeout produces more keys but
/// we intentionally only parse the ones that map to XMP fields.
#[derive(Debug, Default, Deserialize)]
struct TakeoutJson {
    title: Option<String>,
    description: Option<String>,
    #[serde(default)]
    people: Vec<TakeoutPerson>,
    #[serde(rename = "photoTakenTime")]
    photo_taken_time: Option<TakeoutTime>,
    #[serde(rename = "geoData")]
    geo_data: Option<TakeoutGeo>,
}

#[derive(Debug, Default, Deserialize)]
struct TakeoutTime {
    timestamp: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct TakeoutGeo {
    latitude: Option<f64>,
    longitude: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct TakeoutPerson {
    name: Option<String>,
}

fn takeout_sidecar_path(media: &Path) -> Option<PathBuf> {
    // Takeout tries two shapes:
    //   IMG_0001.jpg.json
    //   IMG_0001.jpg(1).json  (for colliding names it suffixes with (N))
    let parent = media.parent()?;
    let name = media.file_name()?.to_string_lossy();
    let candidates = [
        parent.join(format!("{name}.json")),
        parent.join(format!("{name}.supplemental-metadata.json")),
    ];
    candidates.into_iter().find(|p| p.exists())
}

fn reconstruct_xmp_from_takeout(media: &Path, existing_sidecar: &Path) -> Result<bool> {
    let json = std::fs::read_to_string(existing_sidecar)?;
    let meta: TakeoutJson = match serde_json::from_str(&json) {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!(?e, path = ?existing_sidecar, "takeout json parse failed");
            return Ok(false);
        }
    };

    let mut fields = XmpFields::default();
    fields.title = meta.title.clone();
    fields.description = meta.description.clone();
    if let Some(t) = &meta.photo_taken_time {
        if let Some(ts) = &t.timestamp {
            if let Ok(secs) = ts.parse::<i64>() {
                fields.date_created = DateTime::from_timestamp(secs, 0);
            }
        }
    }
    if let Some(g) = meta.geo_data {
        if let (Some(lat), Some(lon)) = (g.latitude, g.longitude) {
            // Google sometimes emits (0.0, 0.0) as "no GPS"; elide.
            if !(lat == 0.0 && lon == 0.0) {
                fields.gps = Some((lat, lon));
            }
        }
    }
    fields.persons = meta
        .people
        .into_iter()
        .filter_map(|p| p.name)
        .collect();
    fields.album = meta
        .description
        .as_deref()
        .and_then(|_| None);

    // Drop-in: write <media>.xmp only if not already present.
    let mut target_xmp = media.to_path_buf();
    let new_ext = match target_xmp.extension().and_then(|e| e.to_str()) {
        Some(e) => format!("{e}.xmp"),
        None => "xmp".to_string(),
    };
    target_xmp.set_extension(new_ext);

    if target_xmp.exists() {
        tracing::debug!(path = ?target_xmp, "xmp sidecar already present — skipping takeout reconstruct");
        return Ok(false);
    }
    let written = write_xmp_sidecar(media, &fields).map(|_| true)?;
    Ok(written)
}

#[async_trait::async_trait]
impl IngestAdapter for GoogleTakeoutAdapter {
    fn kind(&self) -> &'static str {
        "google_takeout"
    }

    async fn scan(&self, root: &Path) -> Result<ScanReport> {
        self.inner.scan(root).await
    }

    async fn ingest(
        &self,
        ctx: &IngestCtx,
        root: &Path,
        source_id: i64,
        progress: Progress,
    ) -> Result<IngestReport> {
        // Pre-pass: materialise Takeout JSON sidecars as XMP so the generic
        // ingest pipeline picks them up uniformly.
        let paths: Vec<PathBuf> = WalkDir::new(root)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().to_path_buf())
            .filter(|p| !p.extension().map(|e| e == "json" || e == "xmp").unwrap_or(false))
            .collect();
        let mut reconstructed = 0u64;
        for media in &paths {
            if let Some(sidecar) = takeout_sidecar_path(media) {
                if reconstruct_xmp_from_takeout(media, &sidecar).unwrap_or(false) {
                    reconstructed += 1;
                }
            }
        }
        tracing::info!(reconstructed, "takeout json → xmp reconstruction complete");
        self.inner.ingest(ctx, root, source_id, progress).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_min_json(path: &Path, json: &str) {
        std::fs::write(path, json).unwrap();
    }

    fn write_jpeg(path: &Path, seed: u8) {
        let img: image::ImageBuffer<image::Rgb<u8>, Vec<u8>> =
            image::ImageBuffer::from_fn(8, 6, |x, y| {
                image::Rgb([(x as u8).wrapping_add(seed), y as u8, 0])
            });
        img.save_with_format(path, image::ImageFormat::Jpeg).unwrap();
    }

    #[test]
    fn takeout_json_reconstructs_xmp() {
        let dir = TempDir::new().unwrap();
        let media = dir.path().join("IMG_0001.JPG");
        let json = dir.path().join("IMG_0001.JPG.json");
        write_jpeg(&media, 5);
        write_min_json(
            &json,
            r#"{"title":"Beach","description":"sunset","photoTakenTime":{"timestamp":"1719849600"},"geoData":{"latitude":36.5732,"longitude":-121.9498}}"#,
        );
        let wrote = reconstruct_xmp_from_takeout(&media, &json).unwrap();
        assert!(wrote);
        let xmp = media.with_extension("JPG.xmp");
        assert!(xmp.exists());
        let body = std::fs::read_to_string(&xmp).unwrap();
        assert!(body.contains("Beach"));
        assert!(body.contains("sunset"));
        assert!(body.contains("exif:GPSLatitude"));
    }

    #[test]
    fn does_not_overwrite_existing_xmp() {
        let dir = TempDir::new().unwrap();
        let media = dir.path().join("IMG_0001.JPG");
        write_jpeg(&media, 5);
        let xmp = media.with_extension("JPG.xmp");
        std::fs::write(&xmp, b"existing sidecar").unwrap();
        let json = dir.path().join("IMG_0001.JPG.json");
        write_min_json(&json, r#"{"title":"other"}"#);
        let wrote = reconstruct_xmp_from_takeout(&media, &json).unwrap();
        assert!(!wrote);
        let body = std::fs::read_to_string(&xmp).unwrap();
        assert_eq!(body, "existing sidecar");
    }

    #[test]
    fn adapter_kind_is_google_takeout() {
        assert_eq!(GoogleTakeoutAdapter::new().kind(), "google_takeout");
    }

    #[test]
    fn takeout_sidecar_path_finds_json() {
        let dir = TempDir::new().unwrap();
        let media = dir.path().join("a.jpg");
        std::fs::write(&media, "").unwrap();
        std::fs::write(dir.path().join("a.jpg.json"), "{}").unwrap();
        let p = takeout_sidecar_path(&media).unwrap();
        assert_eq!(p, dir.path().join("a.jpg.json"));
    }
}
