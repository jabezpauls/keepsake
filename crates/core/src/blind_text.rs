//! Blind-indexed full-text search over OCR'd image text (D5).
//!
//! `asset_text` is an FTS5 virtual table declared in DDL v1. Each row
//! stores `(asset_id, token_blind)` where `token_blind` is a keyed hash
//! of a single plaintext token. The keying material — `search_key` —
//! is deterministically derived from the user's `master_key`, so the
//! index is readable only under an unlocked keystore.
//!
//! Surface:
//! - [`derive_search_key`] — KDF the per-user search key.
//! - [`tokenize`] — split + normalise a text block the same way OCR
//!   output should be tokenised.
//! - [`token_blind`] — HMAC one token with the search key. Returns
//!   32 lowercase hex chars (FTS5 friendly).
//! - [`index_asset_tokens`] — replace the asset's tokens.
//! - [`search_assets`] — tokenise a query, blind each word, AND-match
//!   against `asset_text`.
//!
//! **Constraints**:
//! - Whole-word matching only. Substring / prefix search is impossible
//!   under blind indexing — an attacker who sees `token_blind` learns
//!   nothing about the plaintext, which is the whole point.
//! - Multi-word queries are AND-combined (every word must match).
//! - Punctuation, diacritics, and case are normalised identically in
//!   indexing + search so the two sides agree.
//!
//! The OCR worker that produces the plaintext lives in
//! `crate::ml::ocr` and is feature-gated on `ml-models`; this module
//! works without weights — it's pure SQL + crypto.

use blake3::Hasher;
use rusqlite::{params, Connection};

use crate::crypto::MasterKey;
use crate::Result;

/// Domain-separation tag for the per-user search key. Bumping this
/// string rotates every blind-token index (old rows stop matching);
/// we'd only do that on a crypto-policy change.
const SEARCH_KEY_LABEL: &[u8] = b"mv.search_key.v1";

/// HMAC-ish keyed hash label. Kept distinct from `SEARCH_KEY_LABEL`
/// so a future multi-purpose derivation from the same base secret
/// can't collide with token hashes.
const TOKEN_BLIND_LABEL: &[u8] = b"mv.search_key.v1.token";

/// Derive the per-user search key from their master key. BLAKE3's
/// `derive_key` mode is KDF-safe under a unique context string, so we
/// use that rather than stacking HMACs.
pub fn derive_search_key(master_key: &MasterKey) -> [u8; 32] {
    let mut out = [0u8; 32];
    let ctx = std::str::from_utf8(SEARCH_KEY_LABEL).unwrap();
    out.copy_from_slice(blake3::derive_key(ctx, master_key.as_bytes()).as_ref());
    out
}

/// Normalise + split a text block into match tokens. Identical pass
/// must be run at index-time and search-time for the two sides to
/// agree. Contract:
/// - Lowercase Unicode.
/// - Drop anything that isn't an alphabetic or numeric char.
/// - De-duplicate — token multiplicity doesn't influence match.
///
/// Diacritics are kept as-is (e.g. `café` ≠ `cafe`). Upstream can
/// fold them via `unicode-normalization` if later testing shows
/// recall suffers; stripping is lossy and I'd rather defer the choice
/// until we have a real fixture.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            cur.extend(c.to_lowercase());
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.sort();
    out.dedup();
    out
}

/// HMAC-shape keyed hash of a single normalised token. Uses BLAKE3's
/// `keyed_hash` for a single round; truncates to 16 bytes (32 hex
/// chars) — large enough that collisions are cosmological, small
/// enough to keep the FTS5 index compact.
pub fn token_blind(search_key: &[u8; 32], token: &str) -> String {
    // Mix a fixed label in so derive-from-same-key for a different
    // purpose can't collide with tokens.
    let mut hasher = Hasher::new_keyed(search_key);
    hasher.update(TOKEN_BLIND_LABEL);
    hasher.update(b"\x00");
    hasher.update(token.as_bytes());
    let full = hasher.finalize();
    let bytes = full.as_bytes();
    let mut s = String::with_capacity(32);
    for &b in &bytes[..16] {
        use std::fmt::Write;
        write!(&mut s, "{:02x}", b).unwrap();
    }
    s
}

/// Replace the `asset_text` row for `asset_id` with a single FTS5
/// entry whose `token_blind` column carries every supplied token
/// space-joined. One-row-per-asset keeps multi-word AND queries
/// working under FTS5's row-local MATCH semantics.
///
/// Empty input clears the row.
pub fn index_asset_tokens(
    conn: &Connection,
    asset_id: i64,
    search_key: &[u8; 32],
    tokens: &[String],
) -> Result<usize> {
    conn.execute(
        "DELETE FROM asset_text WHERE asset_id = ?1",
        params![asset_id],
    )?;
    if tokens.is_empty() {
        return Ok(0);
    }
    let blob: String = tokens
        .iter()
        .map(|t| token_blind(search_key, t))
        .collect::<Vec<_>>()
        .join(" ");
    conn.execute(
        "INSERT INTO asset_text (asset_id, token_blind) VALUES (?1, ?2)",
        params![asset_id, &blob],
    )?;
    Ok(tokens.len())
}

/// Find asset ids that match every token in `query`. Returns in
/// `asset_id` ascending order. Empty query returns the empty set (an
/// empty AND would otherwise match everything, which is usually not
/// what the caller wanted).
pub fn search_assets(
    conn: &Connection,
    search_key: &[u8; 32],
    query: &str,
    limit: u32,
) -> Result<Vec<i64>> {
    let tokens = tokenize(query);
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    // FTS5 MATCH: each blind token is a column search. AND across
    // tokens is the default `token1 token2 token3` syntax.
    let blind_words: Vec<String> = tokens.iter().map(|t| token_blind(search_key, t)).collect();
    let match_expr = blind_words.join(" ");
    let mut stmt = conn.prepare(
        r"SELECT DISTINCT asset_id FROM asset_text WHERE asset_text MATCH ?1
          ORDER BY asset_id ASC LIMIT ?2",
    )?;
    let rows: Vec<i64> = stmt
        .query_map(params![match_expr, limit], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn open() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        db::migrate::apply(&conn).unwrap();
        conn
    }

    #[test]
    fn derive_is_deterministic_per_master_key() {
        let mk1 = MasterKey::from_bytes([7u8; 32]);
        let mk2 = MasterKey::from_bytes([7u8; 32]);
        let mk3 = MasterKey::from_bytes([8u8; 32]);
        assert_eq!(derive_search_key(&mk1), derive_search_key(&mk2));
        assert_ne!(derive_search_key(&mk1), derive_search_key(&mk3));
    }

    #[test]
    fn tokenize_normalises_and_dedupes() {
        let t = tokenize("Hello, HELLO world! hello");
        assert_eq!(t, vec!["hello".to_string(), "world".to_string()]);
    }

    #[test]
    fn tokenize_strips_punctuation_and_keeps_digits() {
        let t = tokenize("Lens-85mm f/1.4");
        // digits and letters kept; `-`, `/`, `.` are separators
        assert!(t.contains(&"lens".to_string()));
        assert!(t.contains(&"85mm".to_string()));
        assert!(t.contains(&"f".to_string()));
        assert!(t.contains(&"1".to_string()));
        assert!(t.contains(&"4".to_string()));
    }

    #[test]
    fn token_blind_is_deterministic_and_key_dependent() {
        let k1 = [0u8; 32];
        let k2 = [1u8; 32];
        assert_eq!(token_blind(&k1, "hello"), token_blind(&k1, "hello"));
        assert_ne!(token_blind(&k1, "hello"), token_blind(&k2, "hello"));
        assert_ne!(token_blind(&k1, "hello"), token_blind(&k1, "world"));
        // 32-char lowercase hex
        assert_eq!(token_blind(&k1, "hello").len(), 32);
        assert!(token_blind(&k1, "hello")
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn index_and_search_roundtrip() {
        let conn = open();
        let mk = MasterKey::from_bytes([3u8; 32]);
        let sk = derive_search_key(&mk);
        // Seed a single asset row so the FK isn't a concern (FTS5 is
        // a virtual table, but we keep the shape honest).
        let tokens_a = tokenize("the quick brown fox");
        let tokens_b = tokenize("lazy dog barks at quick fox");
        index_asset_tokens(&conn, 1, &sk, &tokens_a).unwrap();
        index_asset_tokens(&conn, 2, &sk, &tokens_b).unwrap();

        // "fox" matches both.
        let hits = search_assets(&conn, &sk, "fox", 10).unwrap();
        assert_eq!(hits, vec![1, 2]);
        // "quick fox" still matches both (AND of both blind tokens).
        let hits = search_assets(&conn, &sk, "quick fox", 10).unwrap();
        assert_eq!(hits, vec![1, 2]);
        // "brown fox" matches only asset 1.
        let hits = search_assets(&conn, &sk, "brown fox", 10).unwrap();
        assert_eq!(hits, vec![1]);
        // Case + punctuation in query still works.
        let hits = search_assets(&conn, &sk, "Quick, FOX!", 10).unwrap();
        assert_eq!(hits, vec![1, 2]);
        // Unknown word matches nothing.
        let hits = search_assets(&conn, &sk, "zebra", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn wrong_key_sees_no_matches() {
        let conn = open();
        let right = derive_search_key(&MasterKey::from_bytes([3u8; 32]));
        let wrong = derive_search_key(&MasterKey::from_bytes([4u8; 32]));
        index_asset_tokens(&conn, 1, &right, &tokenize("hello world")).unwrap();
        assert!(search_assets(&conn, &wrong, "hello", 10)
            .unwrap()
            .is_empty());
        assert_eq!(search_assets(&conn, &right, "hello", 10).unwrap(), vec![1]);
    }

    #[test]
    fn re_indexing_replaces_prior_tokens() {
        let conn = open();
        let sk = derive_search_key(&MasterKey::from_bytes([1u8; 32]));
        index_asset_tokens(&conn, 7, &sk, &tokenize("old text here")).unwrap();
        index_asset_tokens(&conn, 7, &sk, &tokenize("new text only")).unwrap();
        assert!(search_assets(&conn, &sk, "old", 10).unwrap().is_empty());
        assert_eq!(search_assets(&conn, &sk, "new", 10).unwrap(), vec![7]);
    }

    #[test]
    fn empty_query_returns_empty() {
        let conn = open();
        let sk = derive_search_key(&MasterKey::from_bytes([1u8; 32]));
        index_asset_tokens(&conn, 1, &sk, &tokenize("some text")).unwrap();
        assert!(search_assets(&conn, &sk, "", 10).unwrap().is_empty());
        assert!(search_assets(&conn, &sk, "   ", 10).unwrap().is_empty());
        assert!(search_assets(&conn, &sk, "!!", 10).unwrap().is_empty());
    }
}
