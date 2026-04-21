//! Phase-1 Acceptance #4 — every envelope API round-trips for input sizes
//! spanning 0 B → 100 MiB; wrong key / tamper yields `Error::KeyOrData`.
//!
//! Also covers Acceptance #3 — vault ciphertext is statistically indistinguishable
//! from random without the password (chi-square on AEAD output).

use mv_core::crypto::{
    keystore, open_blob_reader, open_from_peer, open_row, seal_blob_writer, seal_for_peer,
    seal_row, unwrap_collection_key, unwrap_file_key, wrap_collection_key, wrap_file_key,
    CollectionKey, FileKey, MasterKey, PeerKeypair, PLAINTEXT_CHUNK_SIZE,
};
use proptest::prelude::*;
use secrecy::SecretString;

fn pw(s: &str) -> SecretString {
    SecretString::from(s.to_string())
}

// ---- Row AEAD --------------------------------------------------------------

proptest! {
    #[test]
    fn row_aead_roundtrip_arbitrary(plaintext in proptest::collection::vec(any::<u8>(), 0..4096),
                                    row_id in any::<u64>(),
                                    key in any::<[u8; 32]>()) {
        let ct = seal_row(&plaintext, row_id, &key).unwrap();
        let pt = open_row(&ct, row_id, &key).unwrap();
        prop_assert_eq!(pt, plaintext);
    }

    #[test]
    fn row_aead_any_single_bit_flip_fails(plaintext in proptest::collection::vec(any::<u8>(), 1..512),
                                          row_id in any::<u64>(),
                                          key in any::<[u8; 32]>(),
                                          idx in any::<u16>(),
                                          bit in 0u8..8) {
        let mut ct = seal_row(&plaintext, row_id, &key).unwrap();
        let i = (idx as usize) % ct.len();
        ct[i] ^= 1 << bit;
        let res = open_row(&ct, row_id, &key);
        prop_assert!(res.is_err(), "flip at byte={} bit={} should fail", i, bit);
    }
}

// ---- Blob streaming --------------------------------------------------------

fn seal_then_open(plaintext: &[u8]) -> Vec<u8> {
    let fk = FileKey::random().unwrap();
    let mut out = Vec::new();
    let mut w = seal_blob_writer(&fk, &mut out).unwrap();
    w.write_all(plaintext).unwrap();
    w.finish().unwrap();
    let mut r = open_blob_reader(&fk, std::io::Cursor::new(&out)).unwrap();
    r.read_to_end().unwrap()
}

#[test]
fn blob_roundtrip_boundary_sizes() {
    let sizes = [
        0usize,
        1,
        PLAINTEXT_CHUNK_SIZE - 1,
        PLAINTEXT_CHUNK_SIZE,
        PLAINTEXT_CHUNK_SIZE + 1,
    ];
    for size in sizes {
        let pt: Vec<u8> = (0..size).map(|i| (i * 17 % 251) as u8).collect();
        assert_eq!(seal_then_open(&pt), pt, "size {}", size);
    }
}

#[test]
#[ignore = "expensive — ~200 MB I/O; run with --ignored in CI nightly"]
fn blob_roundtrip_100mib() {
    let size = 100 * 1024 * 1024;
    let pt: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    let fk = FileKey::random().unwrap();
    let mut out = Vec::with_capacity(size + 1024);
    let mut w = seal_blob_writer(&fk, &mut out).unwrap();
    // Feed in 1 MiB slices to exercise the internal chunk buffer.
    for chunk in pt.chunks(1024 * 1024) {
        w.write_all(chunk).unwrap();
    }
    w.finish().unwrap();

    let mut r = open_blob_reader(&fk, std::io::Cursor::new(&out)).unwrap();
    let round = r.read_to_end().unwrap();
    assert_eq!(round.len(), pt.len());
    assert_eq!(&round[..], &pt[..]);
}

// ---- Peer sealed box -------------------------------------------------------

#[test]
fn peer_roundtrip_binary_payload() {
    let kp = PeerKeypair::generate().unwrap();
    let payload: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let ct = seal_for_peer(&payload, &kp.public).unwrap();
    let pt = open_from_peer(&ct, &kp).unwrap();
    assert_eq!(pt, payload);
}

#[test]
fn peer_wrong_keypair_fails() {
    let kp = PeerKeypair::generate().unwrap();
    let other = PeerKeypair::generate().unwrap();
    let ct = seal_for_peer(b"not for you", &kp.public).unwrap();
    assert!(open_from_peer(&ct, &other).is_err());
}

// ---- Wrapping key round-trip ----------------------------------------------

#[test]
fn key_wrap_chain_end_to_end() {
    let (record, _unlocked) = keystore::create_user("u", &pw("a-very-long-password-xyz")).unwrap();
    let unlocked = keystore::unlock(&record, &pw("a-very-long-password-xyz"), 1).unwrap();

    // master ← KEK has already been exercised in create/unlock; check collection/file paths.
    let ck = CollectionKey::random().unwrap();
    let wck = wrap_collection_key(&ck, &unlocked.master_key).unwrap();
    let ck2 = unwrap_collection_key(&wck, &unlocked.master_key).unwrap();
    assert_eq!(ck.as_bytes(), ck2.as_bytes());

    let fk = FileKey::random().unwrap();
    let wfk = wrap_file_key(&fk, &ck).unwrap();
    let fk2 = unwrap_file_key(&wfk, &ck).unwrap();
    assert_eq!(fk.as_bytes(), fk2.as_bytes());

    // Wrong master/collection keys produce KeyOrData.
    let mk2 = MasterKey::random().unwrap();
    assert!(unwrap_collection_key(&wck, &mk2).is_err());
    let ck3 = CollectionKey::random().unwrap();
    assert!(unwrap_file_key(&wfk, &ck3).is_err());
}

// ---- Adversarial ciphertext randomness (Acceptance #3) ---------------------

/// A weak-but-sensitive chi-square test on a large ciphertext sample. The
/// expectation is that per-byte distribution across a random AEAD stream is
/// uniform. Under uniformity the chi-square statistic over 256 bins with
/// N=size samples has mean ≈ 255; we assert a generous upper bound so the
/// test is flake-free.
fn chi_square_bytes(bytes: &[u8]) -> f64 {
    let n = bytes.len() as f64;
    let expected = n / 256.0;
    let mut counts = [0u64; 256];
    for &b in bytes {
        counts[b as usize] += 1;
    }
    counts
        .iter()
        .map(|&c| {
            let d = c as f64 - expected;
            d * d / expected
        })
        .sum()
}

#[test]
fn adversarial_ciphertext_indistinguishable_from_random() {
    // Simulate a 50-asset vault: seal 50 x ~100 KiB plaintexts into blobs + rows.
    // Concatenate the ciphertext (excluding the 4-byte magic headers, which are
    // literally "MVV1" and deterministically non-uniform) and run chi-square.
    let (record, unlocked) = keystore::create_user("a", &pw("never-guess-this-pw-12345")).unwrap();
    let ck = CollectionKey::random().unwrap();
    let _ = wrap_collection_key(&ck, &unlocked.master_key).unwrap();

    let mut body = Vec::with_capacity(50 * 100 * 1024);

    // 50 blobs.
    for i in 0u32..50 {
        let fk = FileKey::random().unwrap();
        let plaintext: Vec<u8> = (0..100 * 1024).map(|j| ((i as usize + j) % 251) as u8).collect();
        let mut out = Vec::with_capacity(plaintext.len() + 512);
        let mut w = seal_blob_writer(&fk, &mut out).unwrap();
        w.write_all(&plaintext).unwrap();
        w.finish().unwrap();
        // Skip the 4-byte MVV1 magic.
        body.extend_from_slice(&out[4..]);
    }

    // 50 AEAD rows too.
    for i in 0u64..50 {
        let ct = seal_row(b"some plaintext metadata that repeats a lot", i, ck.as_bytes()).unwrap();
        body.extend_from_slice(&ct);
    }

    // And the wrapped keys themselves.
    body.extend_from_slice(&record.wrapped_master_key);
    body.extend_from_slice(&record.wrapped_identity_sk);
    body.extend_from_slice(&record.wrapped_iroh_sk);
    body.extend_from_slice(&record.hidden_wrapped_master_key);
    body.extend_from_slice(&record.username_ct);

    let stat = chi_square_bytes(&body);
    // Chi-square with 255 dof: 99.9th percentile ≈ 330, 99.99th ≈ 347.
    // We give ourselves plenty of headroom to keep the test flake-free.
    assert!(
        stat < 400.0,
        "chi-square {:.2} too high — ciphertext bytes are not uniform enough",
        stat
    );
}
