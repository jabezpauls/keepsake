//! XMP sidecar round-trip (phase-1-foundation.md §9 #6).
//!
//! Write an XMP sidecar next to a media file, read it back, assert the
//! non-trivial fields survive. The core promise is that a user's edits —
//! dates, GPS, titles — are portable back out of the vault.

use chrono::{TimeZone, Utc};
use mv_core::ingest::sidecar::{read_xmp_sidecar, write_xmp_sidecar, XmpFields};
use tempfile::TempDir;

#[test]
fn xmp_round_trip_preserves_edits() {
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("IMG_0001.JPG");
    // The writer needs a real file to sit next to; content doesn't matter.
    std::fs::write(&media, b"fake jpeg").unwrap();

    let before = XmpFields {
        title: Some("Sunset over Monterey Bay".into()),
        description: Some("golden hour, low tide".into()),
        date_created: Some(Utc.with_ymd_and_hms(2024, 9, 21, 18, 5, 0).unwrap()),
        gps: Some((36.5732, -121.9498)),
        persons: vec!["Alice".into(), "Bob".into()],
        album: Some("Beach 2024".into()),
        ..Default::default()
    };

    write_xmp_sidecar(&media, &before).unwrap();
    let after = read_xmp_sidecar(&media)
        .unwrap()
        .expect("sidecar should exist");

    assert_eq!(after.title, before.title);
    assert_eq!(after.description, before.description);
    assert_eq!(after.date_created, before.date_created);
    let (la, lo) = after.gps.unwrap();
    assert!((la - 36.5732).abs() < 0.0001);
    assert!((lo - -121.9498).abs() < 0.0001);
    assert_eq!(after.persons, before.persons);
    assert_eq!(after.album, before.album);
}
