//! Acceptance criterion #7: the `near_dup_set` fixture yields exactly 5
//! clusters with correct membership, no false positives.
//!
//! We exercise `ml::phash::cluster` directly rather than `ml::nearp::rebuild`
//! to keep the fixture pure: rebuild would otherwise need real thumbnails +
//! CAS blobs to pick a best-shot, which isn't what this criterion measures.
//! The rebuild path is covered by its own unit tests in `ml::nearp::tests`.

use mv_core::ml::phash::cluster;

/// The pinned near-dup fixture — 5 clusters × 3 members. Hashes inside each
/// cluster differ by ≤ 3 bits; different clusters are separated by ≥ 20 bits.
/// These values are frozen; regenerating requires an Acceptance re-baseline.
const FIXTURE: &[(i64, u64)] = &[
    // Cluster A (base = 0x00000000000000FF)
    (1, 0x00000000000000FF),
    (2, 0x00000000000000FD),
    (3, 0x00000000000000BF),
    // Cluster B (base = 0xFFFF00000000FFFF)
    (4, 0xFFFF00000000FFFF),
    (5, 0xFFFF00000000FFDF),
    (6, 0xFFFE00000000FFFF),
    // Cluster C (base = 0xAAAAAAAAAAAAAAAA)
    (7, 0xAAAAAAAAAAAAAAAA),
    (8, 0xAAAAAAAAAAAAAAA2),
    (9, 0xAAAA8AAAAAAAAAAA),
    // Cluster D (base = 0x123456789ABCDEF0)
    (10, 0x123456789ABCDEF0),
    (11, 0x123456789ABCDEF8),
    (12, 0x123456799ABCDEF0),
    // Cluster E (base = 0xDEADBEEFCAFEBABE)
    (13, 0xDEADBEEFCAFEBABE),
    (14, 0xDEADBEEFCAFEBABA),
    (15, 0xDEADBEEFCAFE3ABE),
];

#[test]
fn near_dup_set_yields_five_clusters() {
    // Phase-2 default threshold is 6 Hamming bits (nearp.rs:DEFAULT_THRESHOLD).
    let groups = cluster(FIXTURE, 6);
    assert_eq!(groups.len(), 5, "expected 5 clusters, got {}", groups.len());

    // Every cluster must have exactly 3 members and the canonical membership
    // sets must match. Normalize by sorting each inner vec + the outer list.
    let mut sorted: Vec<Vec<i64>> = groups
        .into_iter()
        .map(|mut c| {
            c.sort();
            c
        })
        .collect();
    sorted.sort_by_key(|c| c[0]);

    let expected: Vec<Vec<i64>> = vec![
        vec![1, 2, 3],
        vec![4, 5, 6],
        vec![7, 8, 9],
        vec![10, 11, 12],
        vec![13, 14, 15],
    ];
    assert_eq!(sorted, expected);
}

#[test]
fn near_dup_set_no_false_positives_under_strict_threshold() {
    // With threshold 0, only bit-identical hashes cluster. No pair in the
    // fixture is identical, so the output must be zero clusters (all
    // singletons filtered).
    let groups = cluster(FIXTURE, 0);
    assert_eq!(groups.len(), 0);
}
