//! XMP sidecar read/write — the user-facing portability layer.
//!
//! The acceptance contract (phase-1-foundation.md §9.6):
//!
//! > the DB is never a roach motel — every metadata edit round-trips to
//! > XMP sidecars on export.
//!
//! We parse a pragmatic subset of the Adobe XMP namespaces — `dc:*`,
//! `xmp:*`, `exif:GPS*`, `photoshop:*`, plus our own `mediavault:*` — and
//! emit standards-compliant XMP that a downstream tool (Lightroom, darktable,
//! digiKam) can read.
//!
//! This is a *hand-rolled* minimal parser, not a full XMP library. It trades
//! coverage for predictability: the fields we care about are extracted, the
//! rest is ignored.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Result;

/// Subset of XMP fields the app round-trips.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct XmpFields {
    /// `dc:title` (single string — we ignore XMP alt/lang wrappers).
    pub title: Option<String>,
    /// `dc:description`.
    pub description: Option<String>,
    /// `dc:subject` — tags / keywords.
    pub tags: Vec<String>,
    /// `xmp:CreateDate` or `photoshop:DateCreated` — whichever is more specific.
    pub date_created: Option<DateTime<Utc>>,
    /// `exif:GPSLatitude` / `exif:GPSLongitude` — decoded decimal degrees.
    pub gps: Option<(f64, f64)>,
    /// `photoshop:Instructions` — used as our user-notes field.
    pub notes: Option<String>,
    /// `mediavault:PersonNames` — people assigned in-app.
    pub persons: Vec<String>,
    /// `mediavault:AlbumName` — source album (for dedupe on re-ingest).
    pub album: Option<String>,
}

// --------- READING ------------------------------------------------------------

fn sidecar_path(original: &Path) -> PathBuf {
    let mut p = original.to_path_buf();
    let new_ext = match p.extension().and_then(|e| e.to_str()) {
        Some(e) => format!("{e}.xmp"),
        None => "xmp".to_string(),
    };
    p.set_extension(new_ext);
    p
}

/// Read the XMP sidecar next to `original` (if present). Returns `Ok(None)`
/// if no sidecar exists.
pub fn read_xmp_sidecar(original: &Path) -> Result<Option<XmpFields>> {
    let path = sidecar_path(original);
    if !path.exists() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&path)?;
    Ok(Some(parse(&body)?))
}

fn parse(body: &str) -> Result<XmpFields> {
    let mut out = XmpFields::default();

    // dc:title, dc:description: nested inside `<rdf:Alt><rdf:li xml:lang="...">text</rdf:li></rdf:Alt>`.
    out.title = extract_alt(body, "dc:title");
    out.description = extract_alt(body, "dc:description");

    // dc:subject: bag of strings.
    out.tags = extract_bag(body, "dc:subject");

    // photoshop:Instructions: a plain attribute or simple text element.
    out.notes = extract_simple(body, "photoshop:Instructions")
        .or_else(|| extract_attr(body, "photoshop:Instructions"));

    // xmp:CreateDate preferred over photoshop:DateCreated.
    let date_str = extract_simple(body, "xmp:CreateDate")
        .or_else(|| extract_attr(body, "xmp:CreateDate"))
        .or_else(|| extract_simple(body, "photoshop:DateCreated"))
        .or_else(|| extract_attr(body, "photoshop:DateCreated"));
    if let Some(s) = date_str {
        if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
            out.date_created = Some(dt.with_timezone(&Utc));
        }
    }

    // exif:GPS — XMP encodes as "47,37.45N" (deg,min+frac ref).
    if let (Some(lat), Some(lon)) = (
        extract_attr(body, "exif:GPSLatitude").or_else(|| extract_simple(body, "exif:GPSLatitude")),
        extract_attr(body, "exif:GPSLongitude").or_else(|| extract_simple(body, "exif:GPSLongitude")),
    ) {
        if let (Some(la), Some(lo)) = (parse_xmp_gps(&lat), parse_xmp_gps(&lon)) {
            out.gps = Some((la, lo));
        }
    }

    // mediavault: custom tags.
    out.persons = extract_bag(body, "mediavault:PersonNames");
    out.album = extract_simple(body, "mediavault:AlbumName")
        .or_else(|| extract_attr(body, "mediavault:AlbumName"));

    Ok(out)
}

/// `<ns:tag>value</ns:tag>`
fn extract_simple(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let i = body.find(&open)?;
    let after = &body[i + open.len()..];
    let j = after.find(&close)?;
    let raw = after[..j].trim();
    Some(decode_xml(raw))
}

/// `<Description ... ns:tag="value" ...>` attribute form.
fn extract_attr(body: &str, tag: &str) -> Option<String> {
    let needle = format!("{tag}=\"");
    let i = body.find(&needle)?;
    let start = i + needle.len();
    let rest = &body[start..];
    let end = rest.find('"')?;
    Some(decode_xml(&rest[..end]))
}

/// `<ns:tag><rdf:Alt><rdf:li ...>text</rdf:li></rdf:Alt></ns:tag>`
fn extract_alt(body: &str, tag: &str) -> Option<String> {
    let wrapper = extract_simple(body, tag)?;
    if let Some(v) = extract_first_li(&wrapper) {
        return Some(v);
    }
    Some(wrapper)
}

/// `<ns:tag><rdf:Bag><rdf:li>...</rdf:li>...</rdf:Bag></ns:tag>` → Vec.
fn extract_bag(body: &str, tag: &str) -> Vec<String> {
    let Some(inner) = extract_simple(body, tag) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut rest = inner.as_str();
    while let Some(start) = rest.find("<rdf:li") {
        let after = &rest[start..];
        // skip to '>'
        let Some(close) = after.find('>') else { break };
        let after = &after[close + 1..];
        let Some(end) = after.find("</rdf:li>") else { break };
        out.push(decode_xml(after[..end].trim()));
        rest = &after[end + "</rdf:li>".len()..];
    }
    out
}

fn extract_first_li(inner: &str) -> Option<String> {
    let start = inner.find("<rdf:li")?;
    let after = &inner[start..];
    let close = after.find('>')?;
    let after = &after[close + 1..];
    let end = after.find("</rdf:li>")?;
    Some(decode_xml(after[..end].trim()))
}

fn decode_xml(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn encode_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn parse_xmp_gps(s: &str) -> Option<f64> {
    // XMP GPS is "deg,min.dec[NSEW]" per the spec. Example: "47,37.45N"
    if s.is_empty() {
        return None;
    }
    let (body, sign) = match s.chars().last().unwrap() {
        'N' | 'E' => (&s[..s.len() - 1], 1.0),
        'S' | 'W' => (&s[..s.len() - 1], -1.0),
        _ => (s, 1.0),
    };
    let mut parts = body.split(',');
    let d: f64 = parts.next()?.trim().parse().ok()?;
    let m: f64 = parts.next()?.trim().parse().ok()?;
    Some(sign * (d + m / 60.0))
}

fn format_xmp_gps(v: f64, axis_ns: bool) -> String {
    // axis_ns = true  → "N/S"
    // axis_ns = false → "E/W"
    let (abs, sign) = if v < 0.0 { (-v, if axis_ns { 'S' } else { 'W' }) } else { (v, if axis_ns { 'N' } else { 'E' }) };
    let deg = abs.trunc();
    let min = (abs - deg) * 60.0;
    format!("{:.0},{:.4}{}", deg, min, sign)
}

// --------- WRITING ------------------------------------------------------------

/// Write `fields` as an XMP sidecar at `<original>.xmp`. Overwrites any
/// existing sidecar.
pub fn write_xmp_sidecar(original: &Path, fields: &XmpFields) -> Result<PathBuf> {
    let path = sidecar_path(original);
    let xml = render(fields);
    std::fs::write(&path, xml.as_bytes())?;
    Ok(path)
}

fn render(f: &XmpFields) -> String {
    let mut attrs = BTreeMap::new();
    if let Some(d) = f.date_created {
        attrs.insert("xmp:CreateDate".to_string(), d.to_rfc3339());
    }
    if let Some(n) = &f.notes {
        attrs.insert("photoshop:Instructions".to_string(), encode_xml(n));
    }
    if let Some((la, lo)) = f.gps {
        attrs.insert("exif:GPSLatitude".to_string(), format_xmp_gps(la, true));
        attrs.insert("exif:GPSLongitude".to_string(), format_xmp_gps(lo, false));
    }
    if let Some(a) = &f.album {
        attrs.insert("mediavault:AlbumName".to_string(), encode_xml(a));
    }

    let mut s = String::new();
    s.push_str("<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n");
    s.push_str("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n");
    s.push_str("  <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n");
    s.push_str("    <rdf:Description rdf:about=\"\"\n");
    s.push_str("        xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n");
    s.push_str("        xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n");
    s.push_str("        xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\"\n");
    s.push_str("        xmlns:exif=\"http://ns.adobe.com/exif/1.0/\"\n");
    s.push_str("        xmlns:mediavault=\"https://mediavault.dev/ns/1.0/\"");
    for (k, v) in &attrs {
        s.push_str(&format!("\n        {k}=\"{v}\""));
    }
    s.push_str(">\n");

    if let Some(t) = &f.title {
        s.push_str(&format!(
            "      <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\n",
            encode_xml(t)
        ));
    }
    if let Some(d) = &f.description {
        s.push_str(&format!(
            "      <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>\n",
            encode_xml(d)
        ));
    }
    if !f.tags.is_empty() {
        s.push_str("      <dc:subject><rdf:Bag>\n");
        for t in &f.tags {
            s.push_str(&format!("        <rdf:li>{}</rdf:li>\n", encode_xml(t)));
        }
        s.push_str("      </rdf:Bag></dc:subject>\n");
    }
    if !f.persons.is_empty() {
        s.push_str("      <mediavault:PersonNames><rdf:Bag>\n");
        for p in &f.persons {
            s.push_str(&format!("        <rdf:li>{}</rdf:li>\n", encode_xml(p)));
        }
        s.push_str("      </rdf:Bag></mediavault:PersonNames>\n");
    }

    s.push_str("    </rdf:Description>\n");
    s.push_str("  </rdf:RDF>\n");
    s.push_str("</x:xmpmeta>\n");
    s.push_str("<?xpacket end=\"w\"?>\n");
    s
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample() -> XmpFields {
        XmpFields {
            title: Some("Beach sunset".into()),
            description: Some("Golden hour over the Pacific".into()),
            tags: vec!["beach".into(), "sunset".into(), "pacific".into()],
            date_created: Some("2024-07-01T18:12:00Z".parse().unwrap()),
            gps: Some((36.5732, -121.9498)),
            notes: Some("note & <special>".into()),
            persons: vec!["Alice".into(), "Bob".into()],
            album: Some("Summer 2024".into()),
        }
    }

    #[test]
    fn round_trip_preserves_subset() {
        let dir = TempDir::new().unwrap();
        let orig = dir.path().join("IMG_0001.JPG");
        std::fs::write(&orig, b"pretend-jpeg").unwrap();

        let fields = sample();
        write_xmp_sidecar(&orig, &fields).unwrap();
        let read = read_xmp_sidecar(&orig).unwrap().unwrap();

        assert_eq!(read.title, fields.title);
        assert_eq!(read.description, fields.description);
        assert_eq!(read.tags, fields.tags);
        assert_eq!(read.date_created, fields.date_created);
        assert_eq!(read.notes, fields.notes);
        assert_eq!(read.persons, fields.persons);
        assert_eq!(read.album, fields.album);
        // GPS rounds to 4 decimal minutes so allow a small epsilon.
        let (la, lo) = read.gps.unwrap();
        assert!((la - 36.5732).abs() < 0.001);
        assert!((lo - (-121.9498)).abs() < 0.001);
    }

    #[test]
    fn missing_sidecar_returns_none() {
        let dir = TempDir::new().unwrap();
        let orig = dir.path().join("no-sidecar.jpg");
        std::fs::write(&orig, b"").unwrap();
        assert!(read_xmp_sidecar(&orig).unwrap().is_none());
    }

    #[test]
    fn parse_tolerates_unknown_namespaces() {
        // A minimal XMP packet using simple elements from an unknown ns.
        let body = r#"<?xpacket begin="﻿"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/">
      <dc:title><rdf:Alt><rdf:li xml:lang="x-default">Hello</rdf:li></rdf:Alt></dc:title>
      <unknown:weird>ignored</unknown:weird>
    </rdf:Description>
  </rdf:RDF>
</x:xmpmeta>"#;
        let f = parse(body).unwrap();
        assert_eq!(f.title.as_deref(), Some("Hello"));
    }

    #[test]
    fn sidecar_path_appends_xmp() {
        let p = sidecar_path(std::path::Path::new("/a/b/c.HEIC"));
        assert_eq!(p.to_string_lossy(), "/a/b/c.HEIC.xmp");
        let p2 = sidecar_path(std::path::Path::new("/no-ext"));
        assert_eq!(p2.to_string_lossy(), "/no-ext.xmp");
    }
}
