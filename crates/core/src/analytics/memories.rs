//! On-this-day memory cards.
//!
//! For each prior year where the user has assets within ±3 days of
//! today's date, emit a "memory" group. Each group surfaces on the
//! home screen as a clickable card that drills into a collection-like
//! view of those assets.
//!
//! This is a read-only surface — memories are computed on the fly
//! rather than persisted as collections. That keeps the feature cheap
//! (no migration, no re-cluster) and lets the user add/remove photos
//! from the underlying assets without our caching drift.
//!
//! See `plans/phase-3-peers-smart.md §8`. `on_this_day` covers the
//! date-symmetric home-screen surface. [`year_in_photos`] summarises
//! every full prior year the user has assets in, and
//! [`person_year_memories`] pulls per-person annual compilations that
//! drill into the People tab. The slideshow MP4 renderer is
//! intentionally deferred — ffmpeg scene-stitching is a separate
//! rabbit hole.

use chrono::{Datelike, Days, NaiveDate};

/// Window (in days) around today's month-day to match against. Set to
/// ±3 so a photo taken 3 days before or after today in a prior year
/// still qualifies.
pub const WINDOW_DAYS: i64 = 3;

/// One memory card: a single prior year with its matching asset ids.
#[derive(Debug, Clone)]
pub struct MemoryGroup {
    /// Year the memories were taken in.
    pub year: i32,
    /// Years ago, relative to `today`. Useful for "1 year ago" / "5
    /// years ago" labels.
    pub years_ago: i32,
    /// Matching asset_ids, sorted.
    pub asset_ids: Vec<i64>,
}

/// Produce memory groups for `today` from `(asset_id, day)` rows
/// covering any prior period. Empty input → empty output. Today's own
/// year is excluded by convention (the surface is "on this day ___
/// years ago"); if callers want current-year memories, they can filter
/// differently upstream.
pub fn on_this_day(today: NaiveDate, assets: &[(i64, i64)]) -> Vec<MemoryGroup> {
    let mut by_year: std::collections::HashMap<i32, Vec<i64>> = std::collections::HashMap::new();
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();

    for (asset_id, day) in assets {
        let Some(date) = epoch.checked_add_days(Days::new((*day).max(0) as u64)) else {
            continue;
        };
        // Same year as today → skip ("this year" isn't a memory).
        if date.year() == today.year() {
            continue;
        }
        // Does |date.md - today.md| ≤ WINDOW_DAYS? We compute by
        // projecting the date's month-day onto today's year and
        // comparing the day-of-year deltas. Handles year boundaries
        // naively — good enough for the ±3-day window.
        let projected = NaiveDate::from_ymd_opt(today.year(), date.month(), date.day());
        let Some(proj) = projected else { continue };
        let delta = (proj.ordinal() as i64 - today.ordinal() as i64).abs();
        // Wrap-around: a photo from Dec 30 should match a Jan 1 today.
        let delta = delta.min(365 - delta);
        if delta <= WINDOW_DAYS {
            by_year.entry(date.year()).or_default().push(*asset_id);
        }
    }

    let mut out: Vec<MemoryGroup> = by_year
        .into_iter()
        .map(|(year, mut ids)| {
            ids.sort_unstable();
            MemoryGroup {
                year,
                years_ago: today.year() - year,
                asset_ids: ids,
            }
        })
        .collect();
    // Most-recent year first.
    out.sort_unstable_by_key(|c| std::cmp::Reverse(c.year));
    out
}

/// One "year in photos" summary card.
#[derive(Debug, Clone)]
pub struct YearInPhotos {
    /// Calendar year the summary covers.
    pub year: i32,
    /// Total distinct assets with a `taken_at_utc_day` in that year.
    pub asset_count: usize,
    /// Up to 6 representative asset ids — picked as the median of each
    /// bimonthly bucket so the card feels temporally spread.
    pub highlights: Vec<i64>,
}

/// For every year up to `today.year() - 1` the user has any assets in,
/// emit one [`YearInPhotos`] card. `today`'s own year is excluded —
/// year-in-photos lands on Jan 1 of the following year.
///
/// Highlights are deterministic per `assets` ordering: we bucket each
/// year's assets into six bimonthly slots and take the median id per
/// bucket. That spreads coverage across the year without needing
/// semantic ranking (CLIP-based picking is a follow-up).
pub fn year_in_photos(today: NaiveDate, assets: &[(i64, i64)]) -> Vec<YearInPhotos> {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    // Bucket assets by (year, bimonth 0..=5).
    let mut by_year: std::collections::HashMap<i32, Vec<(usize, i64)>> =
        std::collections::HashMap::new();
    for (asset_id, day) in assets {
        let Some(date) = epoch.checked_add_days(Days::new((*day).max(0) as u64)) else {
            continue;
        };
        if date.year() >= today.year() {
            continue;
        }
        let bimonth = ((date.month() - 1) / 2) as usize; // 0..=5
        by_year
            .entry(date.year())
            .or_default()
            .push((bimonth, *asset_id));
    }

    let mut out: Vec<YearInPhotos> = by_year
        .into_iter()
        .map(|(year, mut rows)| {
            let asset_count = rows.len();
            rows.sort_unstable();
            let mut highlights = Vec::with_capacity(6);
            for b in 0..6 {
                let bucket: Vec<i64> = rows
                    .iter()
                    .filter(|(bm, _)| *bm == b)
                    .map(|(_, id)| *id)
                    .collect();
                if !bucket.is_empty() {
                    highlights.push(bucket[bucket.len() / 2]);
                }
            }
            YearInPhotos {
                year,
                asset_count,
                highlights,
            }
        })
        .collect();
    out.sort_unstable_by_key(|c| std::cmp::Reverse(c.year));
    out
}

/// One "Person X in year Y" card.
#[derive(Debug, Clone)]
pub struct PersonYearMemory {
    pub person_id: i64,
    pub year: i32,
    /// Every asset id that has a face row for this person and was
    /// taken in this year. Sorted ascending.
    pub asset_ids: Vec<i64>,
}

/// Group `(person_id, asset_id, taken_day)` rows into per-(person,
/// year) buckets. A person with assets across three years yields
/// three cards. Cards with fewer than `min_assets` assets are
/// dropped — a "year with Ami" that has one photo is noise, not a
/// memory.
///
/// Excludes `today`'s year for the same reason `on_this_day` does.
pub fn person_year_memories(
    today: NaiveDate,
    rows: &[(i64, i64, i64)],
    min_assets: usize,
) -> Vec<PersonYearMemory> {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    let mut by_key: std::collections::HashMap<(i64, i32), Vec<i64>> =
        std::collections::HashMap::new();
    for (person_id, asset_id, day) in rows {
        let Some(date) = epoch.checked_add_days(Days::new((*day).max(0) as u64)) else {
            continue;
        };
        if date.year() >= today.year() {
            continue;
        }
        by_key
            .entry((*person_id, date.year()))
            .or_default()
            .push(*asset_id);
    }
    let mut out: Vec<PersonYearMemory> = by_key
        .into_iter()
        .filter_map(|((person_id, year), mut ids)| {
            ids.sort_unstable();
            ids.dedup();
            if ids.len() < min_assets {
                return None;
            }
            Some(PersonYearMemory {
                person_id,
                year,
                asset_ids: ids,
            })
        })
        .collect();
    // Sort by person then most-recent year.
    out.sort_unstable_by(|a, b| {
        a.person_id
            .cmp(&b.person_id)
            .then_with(|| b.year.cmp(&a.year))
    });
    out
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> i64 {
        let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        (date - epoch).num_days()
    }

    #[test]
    fn same_day_prior_year_matches() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let assets = vec![
            (10, day(2024, 6, 15)),
            (20, day(2023, 6, 15)),
            (30, day(2020, 6, 15)),
        ];
        let groups = on_this_day(today, &assets);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].year, 2024);
        assert_eq!(groups[1].year, 2023);
        assert_eq!(groups[2].year, 2020);
        assert_eq!(groups[2].years_ago, 6);
    }

    #[test]
    fn within_three_days_matches() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let assets = vec![
            (1, day(2024, 6, 12)), // 3 days before
            (2, day(2024, 6, 18)), // 3 days after
            (3, day(2024, 6, 20)), // 5 days after, should NOT match
        ];
        let groups = on_this_day(today, &assets);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].asset_ids, vec![1, 2]);
    }

    #[test]
    fn current_year_excluded() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let assets = vec![(99, day(2026, 6, 15))];
        assert!(on_this_day(today, &assets).is_empty());
    }

    #[test]
    fn new_year_wraparound() {
        // Jan 1 today; a Dec 30 photo from prior years should match
        // (2-day distance across year boundary).
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let assets = vec![(1, day(2024, 12, 30))];
        let groups = on_this_day(today, &assets);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].asset_ids, vec![1]);
    }

    #[test]
    fn empty_input() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        assert!(on_this_day(today, &[]).is_empty());
    }

    #[test]
    fn year_in_photos_buckets_by_bimonth() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        // 2024: one asset per month → 12 assets → 6 highlights.
        let mut assets = Vec::new();
        for (i, m) in (1..=12u32).enumerate() {
            assets.push((100 + i as i64, day(2024, m, 15)));
        }
        let cards = year_in_photos(today, &assets);
        assert_eq!(cards.len(), 1);
        let card = &cards[0];
        assert_eq!(card.year, 2024);
        assert_eq!(card.asset_count, 12);
        assert_eq!(card.highlights.len(), 6);
    }

    #[test]
    fn year_in_photos_skips_current_year() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let assets = vec![(1, day(2026, 1, 1)), (2, day(2025, 1, 1))];
        let cards = year_in_photos(today, &assets);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].year, 2025);
    }

    #[test]
    fn year_in_photos_sparse_year_still_cards() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let assets = vec![(1, day(2024, 3, 5)), (2, day(2024, 3, 6))];
        let cards = year_in_photos(today, &assets);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].asset_count, 2);
        // Only one bimonthly bucket populated → one highlight.
        assert_eq!(cards[0].highlights.len(), 1);
    }

    #[test]
    fn person_year_memories_basic() {
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        // Person 1 has 3 photos in 2023, 2 photos in 2024.
        // Person 2 has 1 photo in 2023 (below min_assets=2).
        let rows = vec![
            (1, 11, day(2023, 3, 1)),
            (1, 12, day(2023, 4, 1)),
            (1, 13, day(2023, 5, 1)),
            (1, 21, day(2024, 3, 1)),
            (1, 22, day(2024, 4, 1)),
            (2, 31, day(2023, 6, 1)),
        ];
        let cards = person_year_memories(today, &rows, 2);
        assert_eq!(cards.len(), 2);
        // Person 1 has both years; person 2 filtered out by min_assets.
        assert_eq!(cards[0].person_id, 1);
        assert_eq!(cards[0].year, 2024);
        assert_eq!(cards[0].asset_ids, vec![21, 22]);
        assert_eq!(cards[1].person_id, 1);
        assert_eq!(cards[1].year, 2023);
        assert_eq!(cards[1].asset_ids, vec![11, 12, 13]);
    }

    #[test]
    fn person_year_memories_excludes_current_year() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let rows = vec![
            (1, 11, day(2026, 3, 1)),
            (1, 12, day(2026, 4, 1)),
            (1, 13, day(2026, 5, 1)),
        ];
        assert!(person_year_memories(today, &rows, 2).is_empty());
    }

    #[test]
    fn person_year_memories_dedups_duplicate_rows() {
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        // Same (person, asset) appearing twice (could happen via a
        // face-row duplicate) should count as one asset.
        let rows = vec![
            (1, 11, day(2023, 3, 1)),
            (1, 11, day(2023, 3, 1)),
            (1, 12, day(2023, 4, 1)),
        ];
        let cards = person_year_memories(today, &rows, 2);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].asset_ids, vec![11, 12]);
    }
}
