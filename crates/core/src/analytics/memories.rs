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
//! See `plans/phase-3-peers-smart.md §8`. The themed compilations
//! (Person × Year, year-in-photos, slideshow renderer) are deferred —
//! this slice is "on-this-day" only.

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
    out.sort_unstable_by(|a, b| b.year.cmp(&a.year));
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
}
