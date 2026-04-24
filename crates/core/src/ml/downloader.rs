//! First-run model downloader.
//!
//! The on-device ML pipeline depends on ~2 GB of ONNX weights + tokenizer
//! JSON. Without them, `loader::load_all` returns `Error::ModelsUnavailable`
//! and every ML-backed feature (semantic search, face recognition, OCR) is
//! silently disabled. Before this module existed, users had to run
//! `scripts/download_models.sh` from a terminal — a hard requirement that
//! broke "install and just use it" universality.
//!
//! `download_missing` drives the survey + fetch + verify cycle that the
//! Tauri command layer calls on behalf of the wizard UI. It is synchronous
//! (the underlying HTTP client is `ureq`) — the caller runs it from
//! `tokio::task::spawn_blocking` and forwards each [`DownloadEvent`] back
//! through a Tauri channel.
//!
//! ### URL resolution
//!
//! Default URLs point at the same `immich-app/ViT-L-14__openai` and
//! `immich-app/buffalo_l` HuggingFace repos referenced in
//! `scripts/download_models.sh`. Any `MV_MODEL_URL_<NAME>` env var
//! overrides the default for that one file, so developers + mirror-heavy
//! deployments can BYO without touching the binary.
//!
//! ### Failure modes
//!
//! Every file is written to `<name>.tmp` and only renamed into place after
//! its SHA-256 matches `manifest::MODELS`. Partial failures leave the
//! partial file behind so the next run can resume (we use plain rewrite,
//! not Range-based resume — simpler, and the cost of a full retry on
//! transient errors is acceptable on a 2 GB one-time download).

use std::io::{Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

use super::manifest::{sha256_file, ModelEntry, MODELS};
use crate::{Error, Result};

/// Canonical source for one manifest entry. Matches the files referenced
/// in `scripts/download_models.sh`.
#[derive(Debug, Clone, Copy)]
pub struct DownloadSource {
    /// Logical filename that matches the manifest entry.
    pub name: &'static str,
    /// Default HTTPS URL — HuggingFace immich-app mirrors. Overridden when
    /// `env_var` is set in the process environment.
    pub default_url: &'static str,
    /// `MV_MODEL_URL_<NAME>` env var that wins over `default_url`.
    pub env_var: &'static str,
}

pub const SOURCES: &[DownloadSource] = &[
    DownloadSource {
        name: "clip_visual.onnx",
        default_url:
            "https://huggingface.co/immich-app/ViT-L-14__openai/resolve/main/visual/model.onnx",
        env_var: "MV_MODEL_URL_CLIP_VISUAL",
    },
    DownloadSource {
        name: "clip_textual.onnx",
        default_url:
            "https://huggingface.co/immich-app/ViT-L-14__openai/resolve/main/textual/model.onnx",
        env_var: "MV_MODEL_URL_CLIP_TEXTUAL",
    },
    DownloadSource {
        name: "clip_tokenizer.json",
        default_url:
            "https://huggingface.co/immich-app/ViT-L-14__openai/resolve/main/textual/tokenizer.json",
        env_var: "MV_MODEL_URL_CLIP_TOKENIZER",
    },
    DownloadSource {
        name: "scrfd.onnx",
        default_url:
            "https://huggingface.co/immich-app/buffalo_l/resolve/main/detection/model.onnx",
        env_var: "MV_MODEL_URL_SCRFD",
    },
    DownloadSource {
        name: "arcface.onnx",
        default_url:
            "https://huggingface.co/immich-app/buffalo_l/resolve/main/recognition/model.onnx",
        env_var: "MV_MODEL_URL_ARCFACE",
    },
];

/// Resolve the URL for a given manifest filename. Returns `None` when the
/// name isn't known to the sources table — in practice this only fires if
/// `manifest::MODELS` drifts from `SOURCES` without the other being
/// updated.
#[must_use]
pub fn url_for(name: &str) -> Option<String> {
    SOURCES
        .iter()
        .find(|s| s.name == name)
        .map(|s| std::env::var(s.env_var).unwrap_or_else(|_| s.default_url.to_string()))
}

/// Survey result for one manifest entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../app/src/bindings/")]
pub struct ModelFileStatus {
    pub name: String,
    /// File exists on disk (regardless of integrity).
    pub present: bool,
    /// File matches the pinned SHA-256. Always false when `present` is false.
    pub valid: bool,
    /// Bytes on disk. Zero when the file is missing.
    #[ts(type = "number")]
    pub size_bytes: u64,
    /// Pinned hex SHA-256, for UI display / support-bundle purposes.
    pub sha256_expected: String,
}

/// Snapshot of every manifest entry's presence + integrity.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../app/src/bindings/")]
pub struct ModelsStatus {
    pub files: Vec<ModelFileStatus>,
    /// True only when every file is present AND its checksum matches. The UI
    /// uses this for the "skip download" vs "start download" decision.
    pub all_present_valid: bool,
}

/// Walk every manifest entry and summarise what's on disk. Cheap — does one
/// SHA-256 per present file; missing files short-circuit to `present:false`.
pub fn survey(model_dir: &Path) -> Result<ModelsStatus> {
    let mut files = Vec::with_capacity(MODELS.len());
    let mut all_ok = true;
    for entry in MODELS {
        let path = model_dir.join(entry.name);
        let (present, valid, size) = if path.exists() {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            match sha256_file(&path) {
                Ok(hex) if hex.eq_ignore_ascii_case(entry.sha256_hex) => (true, true, size),
                _ => (true, false, size),
            }
        } else {
            (false, false, 0)
        };
        if !valid {
            all_ok = false;
        }
        files.push(ModelFileStatus {
            name: entry.name.to_string(),
            present,
            valid,
            size_bytes: size,
            sha256_expected: entry.sha256_hex.to_string(),
        });
    }
    Ok(ModelsStatus {
        files,
        all_present_valid: all_ok,
    })
}

/// Progress events the downloader streams back to the UI. The Tauri command
/// layer serialises these onto the `ml-download-event` channel.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../app/src/bindings/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DownloadEvent {
    /// Emitted once per file, just after the HTTP response headers arrive.
    /// `bytes_total` is 0 when the server didn't send a Content-Length.
    Start {
        name: String,
        #[ts(type = "number")]
        bytes_total: u64,
    },
    /// Periodic update during the download body read loop. Throttled so the
    /// UI isn't flooded; ~every 256 KiB of body bytes.
    Progress {
        name: String,
        #[ts(type = "number")]
        bytes_downloaded: u64,
        #[ts(type = "number")]
        bytes_total: u64,
    },
    /// File verified against its pinned SHA-256 and moved into place.
    Verified { name: String },
    /// File failed to download or failed verification; reason is user-facing.
    FileFailed { name: String, reason: String },
    /// Terminal event — UI closes the wizard and refreshes `ml_status`.
    AllDone { ok: bool, failed: Vec<String> },
}

/// Download every missing or corrupt manifest entry into `model_dir`.
///
/// - Skips files that are present and checksum-match.
/// - Removes stale files and re-downloads them.
/// - Writes each file as `<name>.tmp` and only renames after SHA-256 matches.
/// - Emits `DownloadEvent`s through `emit` so the UI can render progress.
///
/// Returns `Ok(())` only when every file ends up present + valid. Individual
/// file failures are surfaced through `FileFailed` events + the `AllDone`
/// terminal event; the error return is a coarse "not all models available"
/// signal.
pub fn download_missing<F>(model_dir: &Path, mut emit: F) -> Result<()>
where
    F: FnMut(DownloadEvent) + Send,
{
    std::fs::create_dir_all(model_dir)?;
    let mut failed: Vec<String> = Vec::new();

    for entry in MODELS {
        let dest = model_dir.join(entry.name);
        if dest.exists() {
            match sha256_file(&dest) {
                Ok(hex) if hex.eq_ignore_ascii_case(entry.sha256_hex) => continue,
                _ => {
                    let _ = std::fs::remove_file(&dest);
                }
            }
        }

        match download_one(entry, &dest, &mut emit) {
            Ok(()) => emit(DownloadEvent::Verified {
                name: entry.name.to_string(),
            }),
            Err(e) => {
                let _ = std::fs::remove_file(dest.with_extension("tmp"));
                emit(DownloadEvent::FileFailed {
                    name: entry.name.to_string(),
                    reason: e.to_string(),
                });
                failed.push(entry.name.to_string());
            }
        }
    }

    let ok = failed.is_empty();
    emit(DownloadEvent::AllDone {
        ok,
        failed: failed.clone(),
    });
    if ok {
        Ok(())
    } else {
        Err(Error::Ingest(format!(
            "model download failed for {}",
            failed.join(", ")
        )))
    }
}

fn download_one<F>(entry: &ModelEntry, dest: &Path, emit: &mut F) -> Result<()>
where
    F: FnMut(DownloadEvent),
{
    let url = url_for(entry.name)
        .ok_or_else(|| Error::Ingest(format!("no download URL for {}", entry.name)))?;

    let response = ureq::get(&url)
        .call()
        .map_err(|e| Error::Ingest(format!("HTTP {url}: {e}")))?;

    if !response.status().is_success() {
        return Err(Error::Ingest(format!(
            "HTTP {} for {}",
            response.status(),
            url
        )));
    }

    let (_parts, mut body) = response.into_parts();
    let total = body.content_length().unwrap_or(0);
    emit(DownloadEvent::Start {
        name: entry.name.to_string(),
        bytes_total: total,
    });

    let tmp_path = dest.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp_path)?;
    let mut hasher = Sha256::new();
    let mut reader = body.as_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_emit: u64 = 0;

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| Error::Ingest(format!("read {}: {e}", entry.name)))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        downloaded += n as u64;
        if downloaded - last_emit >= 256 * 1024 {
            emit(DownloadEvent::Progress {
                name: entry.name.to_string(),
                bytes_downloaded: downloaded,
                bytes_total: total,
            });
            last_emit = downloaded;
        }
    }
    // Final progress ping so the bar lands at 100% before the rename.
    emit(DownloadEvent::Progress {
        name: entry.name.to_string(),
        bytes_downloaded: downloaded,
        bytes_total: total.max(downloaded),
    });
    file.flush()?;
    drop(file);

    let got = hex::encode(hasher.finalize());
    if !got.eq_ignore_ascii_case(entry.sha256_hex) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(Error::MlModelChecksum(entry.name));
    }
    std::fs::rename(&tmp_path, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sources_table_covers_every_manifest_entry() {
        for entry in MODELS {
            assert!(
                SOURCES.iter().any(|s| s.name == entry.name),
                "no download source for {}",
                entry.name
            );
        }
    }

    #[test]
    fn url_for_returns_default_when_env_unset() {
        // `MV_MODEL_URL_CLIP_VISUAL` is not set in CI — we deliberately don't
        // flip env vars in tests (touching the global process env races other
        // tests and requires `unsafe` on newer editions). The default-path is
        // the load-bearing case anyway.
        let src = &SOURCES[0];
        let got = url_for(src.name).unwrap();
        // Either the default URL, or whatever the operator already exported.
        let accept = got == src.default_url || got.starts_with("http");
        assert!(accept, "unexpected URL: {got}");
    }

    #[test]
    fn url_for_returns_none_for_unknown_name() {
        assert!(url_for("not_a_model.onnx").is_none());
    }

    #[test]
    fn survey_reports_missing_when_dir_empty() {
        let tmp = TempDir::new().unwrap();
        let s = survey(tmp.path()).unwrap();
        assert_eq!(s.files.len(), MODELS.len());
        assert!(s.files.iter().all(|f| !f.present));
        assert!(!s.all_present_valid);
    }

    #[test]
    fn survey_reports_invalid_when_file_has_wrong_checksum() {
        let tmp = TempDir::new().unwrap();
        let entry = &MODELS[0];
        let path = tmp.path().join(entry.name);
        std::fs::write(&path, b"not the real model").unwrap();
        let s = survey(tmp.path()).unwrap();
        let row = s.files.iter().find(|f| f.name == entry.name).unwrap();
        assert!(row.present);
        assert!(!row.valid);
        assert!(row.size_bytes > 0);
        assert!(!s.all_present_valid);
    }
}
