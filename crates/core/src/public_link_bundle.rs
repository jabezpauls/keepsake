//! D7a: public-link *bundle* — a single self-contained HTML file
//! that decrypts + renders a shared album in any browser.
//!
//! Why a bundle vs. a hosting peer?
//!
//! The plan's vision for D7 was `https://<relay>/s/<pub_id>` with
//! the serving peer answering HTTP. That requires either an
//! HTTP-over-iroh gateway or a conventional relay running in front of
//! iroh — both are their own projects. A static bundle sidesteps
//! the deployment question: the sender runs "export", gets an
//! `index.html`, and ships it via Signal / email / AirDrop / a
//! GitHub gist / whatever. The recipient double-clicks the file.
//! Works offline. Works behind corporate NAT. Works on any OS with a
//! browser.
//!
//! Crypto shape (mirrors what a future HTTP serving peer would do):
//! - Each asset's thumbnail is re-encrypted under a fresh
//!   `viewer_key` (32 bytes) using libsodium secretbox. The original
//!   collection_key stays on the sender — the bundle is a one-shot
//!   hand-off.
//! - `viewer_key` travels in either the URL fragment (no password)
//!   or wrapped under `Argon2id(password, salt)` (inside the HTML).
//! - `expires_at` is checked by the viewer JS against
//!   `Date.now() / 1000`.
//!
//! The module builds the manifest (pure Rust, testable) and renders
//! the HTML (string template + embedded viewer JS). The viewer JS
//! uses DOM APIs rather than `innerHTML` assignment for untrusted
//! content so album names + viewer input can't be smuggled into
//! executable script.

use serde::{Deserialize, Serialize};

use crate::crypto::envelope::wrap_with_key;
use crate::public_link::VIEWER_KEY_LEN;
use crate::Result;

/// One asset entry in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleAsset {
    pub id: i64,
    pub mime: String,
    /// Base64 of `nonce || ciphertext` for the thumbnail, sealed
    /// under the viewer key via libsodium secretbox.
    pub thumb_b64: String,
}

/// Full manifest written into the exported HTML as a JSON
/// `<script type="application/json">` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    pub version: u32,
    pub pub_id: String,
    pub has_password: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub salt_b64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrapped_key_b64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    pub assets: Vec<BundleAsset>,
}

/// Seal a thumbnail plaintext under the viewer key and base64-encode
/// the wrapped bytes.
pub fn seal_thumb_for_bundle(plain: &[u8], viewer_key: &[u8; VIEWER_KEY_LEN]) -> Result<String> {
    let wrapped = wrap_with_key(plain, viewer_key)?;
    Ok(b64_encode(&wrapped))
}

/// Render a `BundleManifest` into a self-contained HTML document the
/// user can share. Viewer JS loads libsodium.js from unpkg on first
/// run; airgap deployments will need to vendor the library (follow-up).
pub fn render_html(manifest: &BundleManifest, album_name: &str) -> Result<String> {
    let manifest_json = serde_json::to_string(manifest).unwrap_or_else(|_| "{}".into());
    let name_escaped = html_escape(album_name);
    Ok(HTML_TEMPLATE
        .replace("__ALBUM_NAME__", &name_escaped)
        .replace("__MANIFEST_JSON__", &manifest_json))
}

fn b64_encode(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    B64.encode(bytes)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Single-file viewer. Placeholders `__ALBUM_NAME__` and
/// `__MANIFEST_JSON__` are replaced at export time.
///
/// The JS intentionally builds the DOM via `createElement` /
/// `textContent` / `appendChild` rather than `innerHTML = ...` so
/// untrusted input (manifest fields, password input, filenames) can't
/// be interpreted as executable script.
const HTML_TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>__ALBUM_NAME__ — Keepsake share</title>
<style>
  :root { color-scheme: dark; }
  body { margin: 0; font: 15px/1.4 -apple-system, BlinkMacSystemFont, sans-serif;
         background: #0b0b0d; color: #eee; }
  header { padding: 1rem 1.5rem; border-bottom: 1px solid #2a2a2e; }
  h1 { margin: 0; font-size: 1.1rem; font-weight: 600; }
  .muted { color: #9a9a9f; font-size: 0.9rem; }
  .pw-form { padding: 2rem; max-width: 420px; margin: 0 auto; }
  .pw-form input { width: 100%; padding: .5rem .7rem; font-size: 1rem;
                   background: #17171b; color: inherit; border: 1px solid #2a2a2e;
                   border-radius: 6px; box-sizing: border-box; }
  .pw-form button { margin-top: .5rem; padding: .5rem 1rem; background: #3b82f6;
                    color: white; border: 0; border-radius: 6px; cursor: pointer; }
  .pw-form .error { color: #fb7185; margin-top: .5rem; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
          gap: .5rem; padding: 1rem 1.5rem; }
  .cell { aspect-ratio: 1; background: #17171b; border-radius: 6px; overflow: hidden; }
  .cell img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .err { padding: 2rem; text-align: center; color: #fb7185; }
</style>
</head>
<body>
<header>
  <h1>__ALBUM_NAME__</h1>
  <div class="muted">Shared via Keepsake — decrypted in your browser.</div>
</header>
<div id="host"></div>
<script id="manifest" type="application/json">__MANIFEST_JSON__</script>
<script src="https://unpkg.com/libsodium-wrappers@0.7.13/dist/browsers/sodium.js"></script>
<script>
(async () => {
  const host = document.getElementById('host');
  const clearHost = () => { while (host.firstChild) host.removeChild(host.firstChild); };
  const showError = (msg) => {
    clearHost();
    const div = document.createElement('div');
    div.className = 'err';
    div.textContent = msg;
    host.appendChild(div);
  };
  const manifest = JSON.parse(document.getElementById('manifest').textContent);

  // Expiry gate — the serving peer would enforce the same check.
  if (manifest.expires_at && manifest.expires_at < Math.floor(Date.now() / 1000)) {
    showError('This link has expired.');
    return;
  }

  await sodium.ready;

  // Resolve the viewer key.
  let viewerKey = null;
  if (manifest.has_password) {
    viewerKey = await promptPassword();
  } else {
    const frag = (window.location.hash || '').replace(/^#/, '').trim();
    if (!frag) {
      showError('Missing viewer key in URL fragment.');
      return;
    }
    viewerKey = base32Decode(frag);
    if (!viewerKey || viewerKey.length !== 32) {
      showError('Invalid viewer key.');
      return;
    }
  }

  renderGrid(viewerKey);

  function promptPassword() {
    return new Promise((resolve) => {
      clearHost();
      const form = document.createElement('form');
      form.className = 'pw-form';
      const p1 = document.createElement('p');
      p1.textContent = 'This share is password-protected.';
      const input = document.createElement('input');
      input.type = 'password';
      input.autofocus = true;
      input.placeholder = 'Password';
      const btn = document.createElement('button');
      btn.type = 'submit';
      btn.textContent = 'Open';
      const err = document.createElement('p');
      err.className = 'error';
      err.hidden = true;
      const note = document.createElement('p');
      note.className = 'muted';
      note.style.marginTop = '1rem';
      note.textContent = 'Decryption happens locally. The password never leaves your browser.';
      form.appendChild(p1);
      form.appendChild(input);
      form.appendChild(btn);
      form.appendChild(err);
      form.appendChild(note);
      host.appendChild(form);

      form.addEventListener('submit', (e) => {
        e.preventDefault();
        const pw = input.value;
        const salt = base64Decode(manifest.salt_b64);
        const wrapped = base64Decode(manifest.wrapped_key_b64);
        try {
          const kek = sodium.crypto_pwhash(
            32,
            pw,
            salt,
            sodium.crypto_pwhash_OPSLIMIT_SENSITIVE,
            sodium.crypto_pwhash_MEMLIMIT_SENSITIVE,
            sodium.crypto_pwhash_ALG_ARGON2ID13
          );
          const nonce = wrapped.slice(0, sodium.crypto_secretbox_NONCEBYTES);
          const ct = wrapped.slice(sodium.crypto_secretbox_NONCEBYTES);
          const vk = sodium.crypto_secretbox_open_easy(ct, nonce, kek);
          resolve(vk);
        } catch (e2) {
          err.textContent = 'Wrong password.';
          err.hidden = false;
        }
      });
    });
  }

  function renderGrid(vk) {
    clearHost();
    const grid = document.createElement('div');
    grid.className = 'grid';
    for (const a of manifest.assets) {
      const cell = document.createElement('div');
      cell.className = 'cell';
      const blob = base64Decode(a.thumb_b64);
      try {
        const nonce = blob.slice(0, sodium.crypto_secretbox_NONCEBYTES);
        const ct = blob.slice(sodium.crypto_secretbox_NONCEBYTES);
        const plain = sodium.crypto_secretbox_open_easy(ct, nonce, vk);
        const url = URL.createObjectURL(new Blob([plain], { type: a.mime }));
        const img = document.createElement('img');
        img.loading = 'lazy';
        img.alt = '';
        img.src = url;
        cell.appendChild(img);
      } catch {
        // Leave the cell empty — the frame still renders as a
        // placeholder so the grid shape is preserved.
      }
      grid.appendChild(cell);
    }
    host.appendChild(grid);
  }

  function base64Decode(s) {
    if (!s) return new Uint8Array();
    const bin = atob(s);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  function base32Decode(s) {
    // RFC4648 without padding, case-insensitive.
    const alpha = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
    const upper = s.toUpperCase().replace(/=+$/g, '');
    const out = [];
    let buf = 0, bits = 0;
    for (const c of upper) {
      const v = alpha.indexOf(c);
      if (v < 0) return null;
      buf = (buf << 5) | v;
      bits += 5;
      if (bits >= 8) {
        bits -= 8;
        out.push((buf >> bits) & 0xff);
      }
    }
    return new Uint8Array(out);
  }
})();
</script>
</body>
</html>
"#;

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_serialises_round_trip() {
        let m = BundleManifest {
            version: 1,
            pub_id: "abc123".into(),
            has_password: true,
            salt_b64: Some("salt==".into()),
            wrapped_key_b64: Some("wrap==".into()),
            expires_at: Some(1700000000),
            assets: vec![BundleAsset {
                id: 42,
                mime: "image/webp".into(),
                thumb_b64: "nope==".into(),
            }],
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: BundleManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pub_id, "abc123");
        assert_eq!(back.assets.len(), 1);
    }

    #[test]
    fn seal_thumb_roundtrips_via_libsodium_secretbox() {
        use crate::crypto::envelope::unwrap_with_key;
        let vk = [7u8; 32];
        let plain = b"fake thumbnail bytes";
        let b64 = seal_thumb_for_bundle(plain, &vk).unwrap();
        use base64::engine::general_purpose::STANDARD as B64;
        use base64::Engine;
        let wrapped = B64.decode(b64).unwrap();
        let recovered = unwrap_with_key(&wrapped, &vk).unwrap();
        assert_eq!(recovered, plain);
    }

    #[test]
    fn render_html_embeds_manifest_and_name() {
        let m = BundleManifest {
            version: 1,
            pub_id: "xy".into(),
            has_password: false,
            salt_b64: None,
            wrapped_key_b64: None,
            expires_at: None,
            assets: vec![],
        };
        let html = render_html(&m, "Beach & Mountain").unwrap();
        // Name escaped.
        assert!(html.contains("Beach &amp; Mountain"));
        // Manifest JSON embedded.
        assert!(html.contains("\"pub_id\":\"xy\""));
        // Viewer JS present.
        assert!(html.contains("sodium.ready"));
    }

    #[test]
    fn html_escaping_defends_against_album_name_injection() {
        let m = BundleManifest {
            version: 1,
            pub_id: "xy".into(),
            has_password: false,
            salt_b64: None,
            wrapped_key_b64: None,
            expires_at: None,
            assets: vec![],
        };
        let html = render_html(&m, "<script>alert(1)</script>").unwrap();
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script>alert(1)"));
    }
}
