//! The CAS writer/reader. See `plans/architecture.md` §3.
//!
//! On-disk layout:
//!
//! ```text
//! <root>/cas/<AA>/<HEX_BLAKE3_OF_PLAINTEXT>
//! <root>/cas/tmp/<uuid>          # staging area for atomic writes
//! <root>/cas/trash/<ts>/<HASH>   # post-GC, retained for N days
//! ```
//!
//! Every blob is `MVV1 || secretstream_header || chunks...` per §2.4.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use uuid::Uuid;

use crate::crypto::{open_blob_reader, seal_blob_writer, BlobReader, FileKey};
use crate::{Error, Result};

const CAS_SUBDIR: &str = "cas";
const TMP_SUBDIR: &str = "tmp";
const TRASH_SUBDIR: &str = "trash";

/// Content-addressed encrypted store.
#[derive(Debug)]
pub struct CasStore {
    root: PathBuf,
}

/// Result of a `CasStore::gc(live)` pass.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    pub kept: usize,
    pub moved_to_trash: usize,
    pub bytes_trashed: u64,
    pub trash_manifest: Option<PathBuf>,
}

impl CasStore {
    /// Open (and if necessary initialise) a CAS store rooted at `root`.
    pub fn open(root: &Path) -> Result<Self> {
        let cas = root.join(CAS_SUBDIR);
        fs::create_dir_all(cas.join(TMP_SUBDIR))?;
        fs::create_dir_all(cas.join(TRASH_SUBDIR))?;
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    /// Encrypt `plaintext` with `file_key`, write it into the CAS keyed by the
    /// plaintext's BLAKE3. Idempotent — if the hash already exists on disk the
    /// call is a no-op and the existing hash is returned.
    pub fn put(&self, plaintext: &[u8], file_key: &FileKey) -> Result<String> {
        self.put_streaming(io::Cursor::new(plaintext), file_key)
            .map(|(hash, _)| hash)
    }

    /// Streaming version: consume `src`, hash it as it flows past, and write
    /// the encrypted bytes to disk atomically. Returns `(hash, bytes_read)`.
    pub fn put_streaming<R: Read>(&self, src: R, file_key: &FileKey) -> Result<(String, u64)> {
        let tmp = self
            .cas_dir()
            .join(TMP_SUBDIR)
            .join(Uuid::new_v4().to_string());
        let file = OpenOptions::new().create_new(true).write(true).open(&tmp)?;

        let mut hasher = blake3::Hasher::new();
        let mut writer = BufWriter::new(file);
        let mut blob = seal_blob_writer(file_key, &mut writer)?;

        let mut buf = vec![0u8; 64 * 1024];
        let mut reader = src;
        let mut total: u64 = 0;
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    drop(blob);
                    drop(writer);
                    let _ = fs::remove_file(&tmp);
                    return Err(Error::Io(e));
                }
            };
            let slice = &buf[..n];
            hasher.update(slice);
            if let Err(e) = blob.write_all(slice) {
                drop(blob);
                drop(writer);
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
            total += n as u64;
        }

        if let Err(e) = blob.finish() {
            drop(writer);
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        let file = writer.into_inner().map_err(|_| Error::Crypto)?;
        file.sync_all()?;
        drop(file);

        let hash_hex = hex::encode(hasher.finalize().as_bytes());
        let final_path = self.blob_path(&hash_hex);

        // Idempotency: if another ingest wrote this blob first, just keep the
        // existing file and discard our temp. (Content is identical under the
        // plaintext hash by construction.)
        if final_path.exists() {
            let _ = fs::remove_file(&tmp);
            return Ok((hash_hex, total));
        }

        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)?;
        }
        // fs::rename is atomic on same-filesystem Unix.
        match fs::rename(&tmp, &final_path) {
            Ok(()) => Ok((hash_hex, total)),
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                Err(Error::Io(e))
            }
        }
    }

    /// Decrypt a blob into memory. Use only for small derivatives; large
    /// media should go through [`Self::open_reader`].
    pub fn get(&self, cas_ref: &str, file_key: &FileKey) -> Result<Vec<u8>> {
        let path = self.blob_path(cas_ref);
        if !path.exists() {
            return Err(Error::NotFound);
        }
        let file = File::open(&path)?;
        let mut reader = open_blob_reader(file_key, BufReader::new(file))?;
        reader.read_to_end()
    }

    /// Streaming decrypt — caller consumes chunks through
    /// [`BlobReader::read_chunk`].
    pub fn open_reader(
        &self,
        cas_ref: &str,
        file_key: &FileKey,
    ) -> Result<BlobReader<BufReader<File>>> {
        let path = self.blob_path(cas_ref);
        if !path.exists() {
            return Err(Error::NotFound);
        }
        let file = File::open(&path)?;
        open_blob_reader(file_key, BufReader::new(file))
    }

    /// Verify a blob's integrity by driving the reader to completion.
    /// Returns [`Error::KeyOrData`] on any authentication failure.
    pub fn verify(&self, cas_ref: &str, file_key: &FileKey) -> Result<()> {
        let mut reader = self.open_reader(cas_ref, file_key)?;
        while reader.read_chunk()?.is_some() {}
        Ok(())
    }

    /// Mark-sweep garbage collection. `live` is the set of hex hashes to keep.
    /// Unreferenced blobs are moved (not deleted) into `cas/trash/<ts>/` with a
    /// timestamped manifest so the operation is reversible for N days.
    pub fn gc(&self, live: &HashSet<String>) -> Result<GcReport> {
        let cas = self.cas_dir();
        let ts = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let trash_root = cas.join(TRASH_SUBDIR).join(&ts);

        let mut report = GcReport::default();
        let mut manifest_entries: Vec<String> = Vec::new();

        for aa_entry in fs::read_dir(&cas)? {
            let aa_entry = aa_entry?;
            let ty = aa_entry.file_type()?;
            if !ty.is_dir() {
                continue;
            }
            let name = aa_entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == TMP_SUBDIR || name_str == TRASH_SUBDIR {
                continue;
            }
            // Only descend into 2-hex-char shard dirs.
            if name_str.len() != 2 || !name_str.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            for blob in fs::read_dir(aa_entry.path())? {
                let blob = blob?;
                if !blob.file_type()?.is_file() {
                    continue;
                }
                let blob_name = blob.file_name().to_string_lossy().into_owned();
                if live.contains(&blob_name) {
                    report.kept += 1;
                    continue;
                }
                let bytes = blob.metadata()?.len();
                fs::create_dir_all(trash_root.join(&*name_str))?;
                let dest = trash_root.join(&*name_str).join(&blob_name);
                fs::rename(blob.path(), &dest)?;
                manifest_entries.push(format!("{}/{}", name_str, blob_name));
                report.moved_to_trash += 1;
                report.bytes_trashed += bytes;
            }
        }

        if report.moved_to_trash > 0 {
            let manifest_path = trash_root.join("MANIFEST.txt");
            let mut mf = File::create(&manifest_path)?;
            mf.write_all(manifest_entries.join("\n").as_bytes())?;
            mf.write_all(b"\n")?;
            mf.sync_all()?;
            report.trash_manifest = Some(manifest_path);
        }

        Ok(report)
    }

    fn cas_dir(&self) -> PathBuf {
        self.root.join(CAS_SUBDIR)
    }

    fn blob_path(&self, hash_hex: &str) -> PathBuf {
        let aa = &hash_hex[..2];
        self.cas_dir().join(aa).join(hash_hex)
    }

    /// The CAS root directory (the parent of `cas/`, `iroh/`, `index.db`,
    /// etc.). The iroh-blobs bridge in `mv-sync` builds its own persistent
    /// store as a sibling at `<root>/iroh/blobs/`.
    pub fn root_path(&self) -> &Path {
        &self.root
    }

    /// Return the on-disk path of the ciphertext file backing `cas_ref`,
    /// plus its size in bytes. Exposed for the Phase-3.2 iroh-blobs
    /// bridge, which content-addresses ciphertext by its own BLAKE3 (the
    /// CAS `cas_ref` is the BLAKE3 of the plaintext, a different hash).
    ///
    /// **Does not decrypt.** Callers reading the returned path see the
    /// full `MVV1` + secretstream wire format from architecture.md §2.4.
    pub fn open_ciphertext_path(&self, cas_ref: &str) -> Result<(PathBuf, u64)> {
        let path = self.blob_path(cas_ref);
        let meta = std::fs::metadata(&path)?;
        Ok((path, meta.len()))
    }

    /// Compute the BLAKE3 hash of the on-disk ciphertext for `cas_ref`.
    /// This is what iroh-blobs indexes by. Cheap-ish (sequential hash of a
    /// file that already lives in page cache after ingest), but still
    /// work the caller must do explicitly — we don't re-hash on every
    /// request.
    pub fn compute_ciphertext_blake3(&self, cas_ref: &str) -> Result<[u8; 32]> {
        use std::io::Read;
        let (path, _) = self.open_ciphertext_path(cas_ref)?;
        let mut f = std::fs::File::open(path)?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(*hasher.finalize().as_bytes())
    }
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn open_ciphertext_path_returns_on_disk_bytes() {
        let (tmp, store) = mk_store();
        let _ = tmp;
        let fk = FileKey::random().unwrap();
        let (cas_ref, _) = store
            .put_streaming(std::io::Cursor::new(b"hello world"), &fk)
            .unwrap();
        let (path, size) = store.open_ciphertext_path(&cas_ref).unwrap();
        assert!(path.exists());
        assert!(size > 0);

        // The ciphertext hash computes deterministically.
        let ct_hash = store.compute_ciphertext_blake3(&cas_ref).unwrap();
        // Must differ from the plaintext BLAKE3 (cas_ref) — ciphertext has
        // the MVV1 magic + random nonce + AEAD overhead.
        let plain_bytes = hex::decode(&cas_ref).unwrap();
        assert_ne!(
            ct_hash.to_vec(),
            plain_bytes,
            "ciphertext hash must differ from plaintext cas_ref hash"
        );
        // Recomputing gives the same bytes.
        assert_eq!(ct_hash, store.compute_ciphertext_blake3(&cas_ref).unwrap());
    }

    #[test]
    fn open_ciphertext_path_missing_is_error() {
        let (tmp, store) = mk_store();
        let _ = tmp;
        let r = store.open_ciphertext_path(&"0".repeat(64));
        assert!(r.is_err(), "missing cas_ref must error");
    }

    fn mk_store() -> (TempDir, CasStore) {
        let dir = TempDir::new().unwrap();
        let store = CasStore::open(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn put_get_round_trip() {
        let (_d, store) = mk_store();
        let fk = FileKey::random().unwrap();
        let pt = b"the quick brown fox";
        let h = store.put(pt, &fk).unwrap();
        // Correct BLAKE3 over plaintext.
        let expected = hex::encode(blake3::hash(pt).as_bytes());
        assert_eq!(h, expected);
        let got = store.get(&h, &fk).unwrap();
        assert_eq!(got, pt);
    }

    #[test]
    fn put_is_idempotent() {
        let (_d, store) = mk_store();
        let fk = FileKey::random().unwrap();
        let h1 = store.put(b"payload", &fk).unwrap();
        let h2 = store.put(b"payload", &fk).unwrap();
        assert_eq!(h1, h2);
        // Only one file on disk under the shard.
        let shard = store.cas_dir().join(&h1[..2]);
        let count = fs::read_dir(shard).unwrap().count();
        assert_eq!(count, 1);
    }

    #[test]
    fn put_different_plaintexts_produce_distinct_hashes() {
        let (_d, store) = mk_store();
        let fk = FileKey::random().unwrap();
        let a = store.put(b"aaaa", &fk).unwrap();
        let b = store.put(b"bbbb", &fk).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn get_wrong_key_fails_opaquely() {
        let (_d, store) = mk_store();
        let fk = FileKey::random().unwrap();
        let other = FileKey::random().unwrap();
        let h = store.put(b"secret", &fk).unwrap();
        let err = store.get(&h, &other).err().unwrap();
        assert!(matches!(err, Error::KeyOrData));
    }

    #[test]
    fn verify_detects_tamper() {
        let (_d, store) = mk_store();
        let fk = FileKey::random().unwrap();
        let h = store.put(b"payload-for-integrity", &fk).unwrap();
        // Corrupt the on-disk blob.
        let path = store.blob_path(&h);
        let mut bytes = fs::read(&path).unwrap();
        let idx = bytes.len() - 5;
        bytes[idx] ^= 0x01;
        fs::write(&path, &bytes).unwrap();
        let err = store.verify(&h, &fk).err().unwrap();
        assert!(matches!(err, Error::KeyOrData));
    }

    #[test]
    fn streaming_put_matches_in_memory() {
        let (_d, store) = mk_store();
        let fk = FileKey::random().unwrap();
        let pt: Vec<u8> = (0u32..500_000).map(|i| (i % 251) as u8).collect();
        let (hash, bytes) = store.put_streaming(std::io::Cursor::new(&pt), &fk).unwrap();
        assert_eq!(bytes as usize, pt.len());
        let h2 = hex::encode(blake3::hash(&pt).as_bytes());
        assert_eq!(hash, h2);
        let round = store.get(&hash, &fk).unwrap();
        assert_eq!(round, pt);
    }

    #[test]
    fn gc_moves_orphans_to_trash_with_manifest() {
        let (_d, store) = mk_store();
        let fk = FileKey::random().unwrap();
        let keep = store.put(b"keeper", &fk).unwrap();
        let drop_ = store.put(b"droppable", &fk).unwrap();

        let mut live = HashSet::new();
        live.insert(keep.clone());

        let report = store.gc(&live).unwrap();
        assert_eq!(report.kept, 1);
        assert_eq!(report.moved_to_trash, 1);
        assert!(report.bytes_trashed > 0);
        let manifest = report.trash_manifest.expect("manifest must exist");
        let contents = fs::read_to_string(&manifest).unwrap();
        assert!(contents.contains(&format!("{}/{}", &drop_[..2], drop_)));

        // `keep` still readable, `drop_` is NotFound.
        assert_eq!(store.get(&keep, &fk).unwrap(), b"keeper");
        assert!(matches!(store.get(&drop_, &fk), Err(Error::NotFound)));
    }

    #[test]
    fn interrupted_write_cleans_up_tmp() {
        // We simulate interruption by constructing a failing Read that errors
        // partway through. The tmp file must be removed.
        let (_d, store) = mk_store();
        let fk = FileKey::random().unwrap();

        struct Boom(usize);
        impl Read for Boom {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0 == 0 {
                    Err(io::Error::other("synthetic read failure"))
                } else {
                    let n = buf.len().min(self.0);
                    buf[..n].fill(b'x');
                    self.0 = self.0.saturating_sub(n);
                    Ok(n)
                }
            }
        }

        let err = store.put_streaming(Boom(512), &fk).err().unwrap();
        assert!(matches!(err, Error::Io(_)));
        // No stale tmp files remain.
        let tmp_dir = store.cas_dir().join(TMP_SUBDIR);
        let count = fs::read_dir(tmp_dir).unwrap().count();
        assert_eq!(count, 0);
    }
}
