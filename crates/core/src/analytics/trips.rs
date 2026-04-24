//! Trip detection: DBSCAN on `(lat, lon, day-of-epoch)` clusters.
//!
//! Design notes (see `plans/phase-3-peers-smart.md §7`):
//!
//! * **Input space** is 3-dimensional: latitude, longitude, and
//!   day-of-epoch. We scale longitude by `cos(lat)` so the Euclidean
//!   metric approximates great-circle distance, then weight the
//!   day-axis so a user-provided time threshold combines sensibly with
//!   the km threshold.
//! * **Eps** is parameterised per user but defaults to 50 km × 3 days.
//!   The combined 3-D eps is the geometric mean after axis rescaling.
//! * **Min samples** defaults to 5 — trips are "a cluster of at least
//!   five photos near each other in space and time". Noise points
//!   (single stragglers) stay un-assigned and never produce a trip.
//! * **Canonical trip id**: deterministic BLAKE3 of the sorted member
//!   asset_ids, so re-running the detector on the same assets always
//!   yields the same id (idempotent re-clustering survives restarts).
//!
//! We operate on already-decrypted `(asset_id, lat, lon, day)` tuples;
//! the caller is responsible for unsealing `asset.gps_ct` under the
//! collection key before invoking us. Keeping decryption outside this
//! module means we can unit-test the clustering on pure numeric data.

use std::collections::HashMap;

/// Radius-of-earth constant used to convert km → degrees.
const KM_PER_DEG: f64 = 111.0;
/// Default spatial radius in km.
pub const DEFAULT_EPS_KM: f64 = 50.0;
/// Default temporal radius in days.
pub const DEFAULT_EPS_DAYS: f64 = 3.0;
/// Default minimum samples per trip cluster.
pub const DEFAULT_MIN_SAMPLES: usize = 5;

/// One geo-tagged asset ready for clustering.
#[derive(Debug, Clone, Copy)]
pub struct GeoPoint {
    pub asset_id: i64,
    pub lat: f64,
    pub lon: f64,
    /// Days since epoch; see `asset.taken_at_utc_day`.
    pub day: i64,
}

/// Clustering configuration.
#[derive(Debug, Clone, Copy)]
pub struct TripParams {
    pub eps_km: f64,
    pub eps_days: f64,
    pub min_samples: usize,
}

impl Default for TripParams {
    fn default() -> Self {
        Self {
            eps_km: DEFAULT_EPS_KM,
            eps_days: DEFAULT_EPS_DAYS,
            min_samples: DEFAULT_MIN_SAMPLES,
        }
    }
}

/// One detected trip.
#[derive(Debug, Clone)]
pub struct Trip {
    /// Deterministic id derived from sorted member asset_ids.
    pub canonical_id: [u8; 32],
    /// Member asset_ids, sorted.
    pub member_asset_ids: Vec<i64>,
    /// Centroid (lat, lon) in degrees. Rounded-off display latitude.
    pub centroid_lat: f64,
    pub centroid_lon: f64,
    /// First + last day-of-epoch covered by this cluster.
    pub start_day: i64,
    pub end_day: i64,
}

/// Cluster `points` into trips. Noise points are dropped (no empty
/// "trip of 1" rows). Empty input → empty output; trivially safe.
pub fn detect_trips(points: &[GeoPoint], params: TripParams) -> Vec<Trip> {
    if points.len() < params.min_samples {
        return Vec::new();
    }

    // Scale the axes so a single Euclidean `eps` covers both the
    // spatial and temporal thresholds. We compute scaled coordinates
    // once, cluster in that space, and unscale for centroid reporting.
    //
    // Spatial: convert (lat, lon) to (lat_km, lon_km) using each
    // point's own latitude to scale longitude. That's approximate but
    // good enough for DBSCAN — within a single cluster the latitude
    // doesn't vary by much.
    let scaled: Vec<[f64; 3]> = points
        .iter()
        .map(|p| {
            let lat_km = p.lat * KM_PER_DEG;
            let lon_km = p.lon * KM_PER_DEG * p.lat.to_radians().cos();
            // Scale days by (eps_km / eps_days) so 3 days and 50 km
            // weigh the same in Euclidean space.
            let day_scaled = (p.day as f64) * (params.eps_km / params.eps_days);
            [lat_km, lon_km, day_scaled]
        })
        .collect();

    let labels = dbscan_euclidean(&scaled, params.eps_km, params.min_samples);

    // Collect labels → member indices.
    let mut groups: HashMap<i32, Vec<usize>> = HashMap::new();
    for (i, &lbl) in labels.iter().enumerate() {
        if lbl >= 0 {
            groups.entry(lbl).or_default().push(i);
        }
    }

    let mut trips = Vec::with_capacity(groups.len());
    for (_label, indices) in groups {
        let mut member_ids: Vec<i64> = indices.iter().map(|&i| points[i].asset_id).collect();
        member_ids.sort_unstable();

        let n = indices.len() as f64;
        let mut lat_sum = 0.0;
        let mut lon_sum = 0.0;
        let mut min_day = i64::MAX;
        let mut max_day = i64::MIN;
        for &i in &indices {
            lat_sum += points[i].lat;
            lon_sum += points[i].lon;
            min_day = min_day.min(points[i].day);
            max_day = max_day.max(points[i].day);
        }

        let mut hasher = blake3::Hasher::new();
        for id in &member_ids {
            hasher.update(&id.to_le_bytes());
        }
        let canonical_id: [u8; 32] = *hasher.finalize().as_bytes();

        trips.push(Trip {
            canonical_id,
            member_asset_ids: member_ids,
            centroid_lat: lat_sum / n,
            centroid_lon: lon_sum / n,
            start_day: min_day,
            end_day: max_day,
        });
    }

    // Deterministic output order: earliest start_day first.
    trips.sort_unstable_by_key(|t| (t.start_day, t.canonical_id));
    trips
}

/// Classic DBSCAN with an L2 metric on generic N-D points. Returns a
/// label per point (−1 = noise, ≥0 = cluster).
///
/// Duplicated from `ml::faces::dbscan_cosine` rather than parameterised
/// because cosine is a different semantic (vectors are normalised,
/// "points" are 3-D geography); keeping them separate stops the distance
/// choice from leaking into the wrong call site.
pub fn dbscan_euclidean(points: &[[f64; 3]], eps: f64, min_samples: usize) -> Vec<i32> {
    let n = points.len();
    let mut labels: Vec<i32> = vec![-2; n]; // -2 = unvisited, -1 = noise
    let mut cluster = 0;
    for i in 0..n {
        if labels[i] != -2 {
            continue;
        }
        let neigh = region_query(points, i, eps);
        if neigh.len() < min_samples {
            labels[i] = -1;
            continue;
        }
        labels[i] = cluster;
        let mut queue = neigh;
        let mut head = 0;
        while head < queue.len() {
            let j = queue[head];
            head += 1;
            if labels[j] == -1 {
                labels[j] = cluster;
            }
            if labels[j] != -2 {
                continue;
            }
            labels[j] = cluster;
            let sub = region_query(points, j, eps);
            if sub.len() >= min_samples {
                for s in sub {
                    if !queue.contains(&s) {
                        queue.push(s);
                    }
                }
            }
        }
        cluster += 1;
    }
    labels
}

fn region_query(points: &[[f64; 3]], i: usize, eps: f64) -> Vec<usize> {
    let p = points[i];
    let mut out = Vec::new();
    let eps2 = eps * eps;
    for (j, q) in points.iter().enumerate() {
        let dx = p[0] - q[0];
        let dy = p[1] - q[1];
        let dz = p[2] - q[2];
        if dx * dx + dy * dy + dz * dz <= eps2 {
            out.push(j);
        }
    }
    out
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(id: i64, lat: f64, lon: f64, day: i64) -> GeoPoint {
        GeoPoint {
            asset_id: id,
            lat,
            lon,
            day,
        }
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(detect_trips(&[], TripParams::default()).is_empty());
    }

    #[test]
    fn far_apart_points_never_cluster() {
        // Tokyo, Paris, NYC — 5 points each, but all three locations
        // far apart → three separate clusters.
        let mut pts = Vec::new();
        for i in 0..5 {
            pts.push(pt(i, 35.68, 139.76, 20_000 + i)); // Tokyo
        }
        for i in 0..5 {
            pts.push(pt(100 + i, 48.85, 2.35, 20_500 + i)); // Paris
        }
        for i in 0..5 {
            pts.push(pt(200 + i, 40.71, -74.00, 20_800 + i)); // NYC
        }
        let trips = detect_trips(&pts, TripParams::default());
        assert_eq!(trips.len(), 3);
        for t in &trips {
            assert_eq!(t.member_asset_ids.len(), 5);
        }
    }

    #[test]
    fn same_place_different_times_split() {
        // Same coords, but visits separated by two months (60 days >
        // eps_days) → two separate trips.
        let mut pts = Vec::new();
        for i in 0..6 {
            pts.push(pt(i, 35.68, 139.76, 20_000 + i));
        }
        for i in 0..6 {
            pts.push(pt(100 + i, 35.68, 139.76, 20_060 + i));
        }
        let trips = detect_trips(&pts, TripParams::default());
        assert_eq!(trips.len(), 2);
    }

    #[test]
    fn same_place_close_times_merge() {
        // One location, two consecutive days → single cluster.
        let mut pts = Vec::new();
        for i in 0..7 {
            pts.push(pt(i, 35.68, 139.76, 20_000 + (i % 2)));
        }
        let trips = detect_trips(&pts, TripParams::default());
        assert_eq!(trips.len(), 1);
        assert_eq!(trips[0].member_asset_ids.len(), 7);
    }

    #[test]
    fn noise_points_excluded() {
        // 5 clustered + 1 lone straggler far away on a different day.
        let mut pts: Vec<_> = (0..5).map(|i| pt(i, 35.68, 139.76, 20_000 + i)).collect();
        pts.push(pt(999, -33.87, 151.21, 30_000)); // Sydney, 2053
        let trips = detect_trips(&pts, TripParams::default());
        assert_eq!(trips.len(), 1);
        assert_eq!(trips[0].member_asset_ids.len(), 5);
        assert!(!trips[0].member_asset_ids.contains(&999));
    }

    #[test]
    fn canonical_id_is_deterministic() {
        let pts: Vec<_> = (0..5).map(|i| pt(i, 35.68, 139.76, 20_000 + i)).collect();
        let t1 = &detect_trips(&pts, TripParams::default())[0];

        // Shuffle input order — canonical id must still match.
        let mut shuffled = pts.clone();
        shuffled.reverse();
        let t2 = &detect_trips(&shuffled, TripParams::default())[0];
        assert_eq!(t1.canonical_id, t2.canonical_id);
    }

    #[test]
    fn below_min_samples_drops_all() {
        // 4 points with default min_samples=5 → no trips.
        let pts: Vec<_> = (0..4).map(|i| pt(i, 35.68, 139.76, 20_000 + i)).collect();
        assert!(detect_trips(&pts, TripParams::default()).is_empty());
    }
}
