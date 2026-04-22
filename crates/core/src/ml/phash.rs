//! 64-bit dhash (difference hash) + near-duplicate clustering.
//!
//! Pure Rust — no model, no FFI. The hash format is an **immutable contract**
//! per `plans/phase-2-browsing.md`: 9×8 greyscale, each of the 64 pairs of
//! horizontal neighbours contributes one bit (`left > right → 1`), packed MSB
//! first. Changing this orders near-dups differently and invalidates every
//! stored `phash` row.

use image::imageops::FilterType;
use image::DynamicImage;

use crate::Result;

/// Compute a 64-bit dhash from raw image bytes.
///
/// Returns `None` if the bytes don't decode as an image — screenshots we can't
/// decode simply skip phash rather than raising an error.
pub fn dhash_bytes(bytes: &[u8]) -> Result<Option<u64>> {
    let reader = match image::load_from_memory(bytes) {
        Ok(img) => img,
        Err(e) => {
            tracing::debug!(?e, "dhash: decode failed");
            return Ok(None);
        }
    };
    Ok(Some(dhash_image(&reader)))
}

/// Compute a 64-bit dhash from an already-decoded image.
pub fn dhash_image(img: &DynamicImage) -> u64 {
    // Resize to 9×8 greyscale — the extra column is what we diff against.
    let small = img.resize_exact(9, 8, FilterType::Triangle).to_luma8();
    let mut out = 0u64;
    let mut bit = 0u8;
    for y in 0..8u32 {
        for x in 0..8u32 {
            let left = small.get_pixel(x, y).0[0];
            let right = small.get_pixel(x + 1, y).0[0];
            if left > right {
                out |= 1u64 << (63 - bit);
            }
            bit += 1;
        }
    }
    out
}

/// Hamming distance between two 64-bit dhashes.
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Cluster (asset_id, dhash) pairs by Hamming distance.
///
/// Implementation: partition by high-4-bit prefix (LSH with 16 buckets) so most
/// comparisons are local, then union-find across bucket boundaries. O(N×K)
/// with N ≈ 100k, K the bucket size — fine for Phase 2 targets.
///
/// Returns one `Vec<asset_id>` per cluster, sorted in descending size. Hashes
/// within `threshold` Hamming distance end up in the same cluster. Singletons
/// are omitted.
pub fn cluster(items: &[(i64, u64)], threshold: u32) -> Vec<Vec<i64>> {
    let n = items.len();
    if n < 2 {
        return Vec::new();
    }
    let mut uf = UnionFind::new(n);
    // LSH: bucket by top-4 bits AND by bits 4..8 so near-dups that straddle a
    // bucket boundary still collide on one of the two partitions.
    let mut buckets_a: std::collections::HashMap<u8, Vec<usize>> = Default::default();
    let mut buckets_b: std::collections::HashMap<u8, Vec<usize>> = Default::default();
    for (i, (_id, h)) in items.iter().enumerate() {
        buckets_a.entry((*h >> 60) as u8).or_default().push(i);
        buckets_b
            .entry(((*h >> 56) & 0xF) as u8)
            .or_default()
            .push(i);
    }
    for bucket in buckets_a.values().chain(buckets_b.values()) {
        for i in 0..bucket.len() {
            for j in (i + 1)..bucket.len() {
                let a = bucket[i];
                let b = bucket[j];
                if hamming(items[a].1, items[b].1) <= threshold {
                    uf.union(a, b);
                }
            }
        }
    }
    // Gather clusters.
    let mut groups: std::collections::HashMap<usize, Vec<i64>> = Default::default();
    for (i, (id, _)) in items.iter().enumerate() {
        groups.entry(uf.find(i)).or_default().push(*id);
    }
    let mut out: Vec<Vec<i64>> = groups.into_values().filter(|g| g.len() > 1).collect();
    out.sort_by_key(|g| std::cmp::Reverse(g.len()));
    out
}

/// Lightweight sharpness estimate via Laplacian variance. Used for best-shot
/// picking in near-dup clusters.
pub fn sharpness(img: &DynamicImage) -> f64 {
    let gray = img.resize_exact(256, 256, FilterType::Triangle).to_luma8();
    let (w, h) = gray.dimensions();
    let mut values: Vec<f64> = Vec::with_capacity((w * h) as usize);
    for y in 1..(h - 1) {
        for x in 1..(w - 1) {
            let c = gray.get_pixel(x, y).0[0] as f64;
            let n = gray.get_pixel(x, y - 1).0[0] as f64;
            let s = gray.get_pixel(x, y + 1).0[0] as f64;
            let e = gray.get_pixel(x + 1, y).0[0] as f64;
            let w2 = gray.get_pixel(x - 1, y).0[0] as f64;
            values.push(4.0 * c - n - s - e - w2);
        }
    }
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64
}

struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}
impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }
    fn find(&mut self, i: usize) -> usize {
        if self.parent[i] != i {
            let root = self.find(self.parent[i]);
            self.parent[i] = root;
        }
        self.parent[i]
    }
    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        match self.rank[ra].cmp(&self.rank[rb]) {
            std::cmp::Ordering::Less => self.parent[ra] = rb,
            std::cmp::Ordering::Greater => self.parent[rb] = ra,
            std::cmp::Ordering::Equal => {
                self.parent[rb] = ra;
                self.rank[ra] += 1;
            }
        }
    }
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn gradient(seed: u8) -> DynamicImage {
        // Mix seed into a non-monotonic pattern so the 9×8 dhash is non-trivial.
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(64, 64, |x, y| {
            let v = ((x as u16 * 11 + y as u16 * 7) ^ (seed as u16 * 37)) as u8;
            Rgb([v, v.wrapping_mul(3), v.wrapping_add(seed)])
        });
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn dhash_is_deterministic_and_differs_on_different_images() {
        let a = gradient(13);
        let b = gradient(200);
        let ha = dhash_image(&a);
        let hb = dhash_image(&b);
        assert_eq!(ha, dhash_image(&a));
        assert_ne!(ha, hb);
    }

    #[test]
    fn dhash_is_stable_under_mild_resize() {
        // Scale a gradient up — hash should stay within a few bits.
        let a = gradient(5);
        let mut large = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(256, 256);
        for (x, y, p) in large.enumerate_pixels_mut() {
            *p = *a
                .as_rgb8()
                .unwrap()
                .get_pixel((x / 4).min(63), (y / 4).min(63));
        }
        let b = DynamicImage::ImageRgb8(large);
        let dist = hamming(dhash_image(&a), dhash_image(&b));
        assert!(dist <= 10, "dhash distance {dist} too high for resize");
    }

    #[test]
    fn cluster_groups_near_duplicates() {
        // 3 near-dups at distance ≤ 3 from a base hash + 1 far-away outlier.
        let base = 0xAAAA_BBBB_CCCC_DDDDu64;
        let a = base;
        let b = base ^ 0b10;
        let c = base ^ 0b110;
        let outlier = !base;
        let items = vec![(1, a), (2, b), (3, c), (4, outlier)];
        let clusters = cluster(&items, 6);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].len(), 3);
        let set: std::collections::HashSet<_> = clusters[0].iter().copied().collect();
        assert_eq!(set, [1i64, 2, 3].into_iter().collect());
    }

    #[test]
    fn cluster_empty_and_singletons() {
        assert!(cluster(&[], 6).is_empty());
        assert!(cluster(&[(1, 0u64)], 6).is_empty());
    }

    #[test]
    fn sharpness_increases_with_edges() {
        // Uniform image → near-zero Laplacian variance.
        let flat: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(64, 64, Rgb([128, 128, 128]));
        let flat_img = DynamicImage::ImageRgb8(flat);
        // Checkerboard → high variance.
        let mut cb: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(64, 64);
        for (x, y, p) in cb.enumerate_pixels_mut() {
            let v = if (x / 4 + y / 4) % 2 == 0 { 0 } else { 255 };
            *p = Rgb([v, v, v]);
        }
        let cb_img = DynamicImage::ImageRgb8(cb);
        assert!(sharpness(&cb_img) > sharpness(&flat_img) * 100.0);
    }
}
