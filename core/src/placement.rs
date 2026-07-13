//! Log placement for expiry-keyed co-deletion units (ADR-0016).
//!
//! A p2panda log is the finest thing the store can delete as a whole, so a
//! log must hold exactly the operations we intend to delete together. This
//! module computes *which bucket* an operation belongs to; mapping a bucket
//! to an actual (opaque, per-author) log id is local authoring state owned by
//! [`crate::domain::JynOperationDomain`].

use serde::{Deserialize, Serialize};

/// The coarse time index for permanent posts: a "post month" of 30 days.
/// Permanent buckets are never drained wholesale — a permanent post is
/// deleted individually — so the index only has to keep bucket sizes sane.
pub const MONTH_SECS: u64 = 30 * 24 * 3600;

/// The fixed lifetime ladder, matching the composer's chips
/// (`lifetimeOptions` in the app). Each chip is a bucket tier whose window
/// granularity equals the chip's own duration, so a bucket over-retains a
/// post by at most one granularity (≤ 100% of its lifetime).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LifetimeChip {
    SixHours,
    OneDay,
    OneWeek,
    OneYear,
}

impl LifetimeChip {
    /// Smallest chip first, so [`Self::for_remaining`] can pick the first fit.
    pub const LADDER: [Self; 4] = [Self::SixHours, Self::OneDay, Self::OneWeek, Self::OneYear];

    pub const fn secs(self) -> u64 {
        match self {
            Self::SixHours => 6 * 3600,
            Self::OneDay => 24 * 3600,
            Self::OneWeek => 7 * 24 * 3600,
            Self::OneYear => 365 * 24 * 3600,
        }
    }

    /// Stable wire-adjacent label used in registry context keys. Never change
    /// a label: it would silently re-key every bucket of that tier.
    pub const fn label(self) -> &'static str {
        match self {
            Self::SixHours => "6h",
            Self::OneDay => "24h",
            Self::OneWeek => "1w",
            Self::OneYear => "1y",
        }
    }

    /// The smallest chip covering a remaining lifetime, saturating at the
    /// top of the ladder. Coarsening is by *remaining* time so a re-homed
    /// snapshot lands in a tier proportional to how long it still has to
    /// live, not to the distance from its original `created_at`.
    pub fn for_remaining(remaining_secs: u64) -> Self {
        Self::LADDER
            .into_iter()
            .find(|chip| remaining_secs <= chip.secs())
            .unwrap_or(Self::OneYear)
    }
}

/// The co-deletion bucket an operation is placed into. Everything in one
/// bucket lives and dies together; GC later drops expired buckets whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogBucket {
    /// Ephemeral content: one bucket per `(chip, expiry window)`, where the
    /// window is `floor(expires_at / chip duration)`. Posts of the same tier
    /// whose expiries fall into the same window share one log. Once the window
    /// passes, GC drops the whole log.
    Expiry { chip: LifetimeChip, window: u64 },
    /// Permanent content, indexed by the month of its creation. A coarse time
    /// index only — deletion here is per-post (by tombstone), never a
    /// window-triggered bucket drop.
    PermanentMonth { month: u64 },
    /// Reactions and comments, indexed by the *reaction's own* creation month
    /// (ADR-0016). A reaction's lifetime is the post's, but the post's expiry
    /// is a foreign, mutable value — keying on it would go stale when the post
    /// author promotes the post, forcing a roll-forward. Keying on the
    /// reaction's own creation is stable, so a lifetime change never re-homes a
    /// reaction; GC instead reaps a reaction reactively when its target post is
    /// gone, and drops the whole month bucket once every reaction in it is
    /// reaped. Kept distinct from [`Self::PermanentMonth`] so a reaction bucket
    /// stays pure (droppable) rather than mixing with permanent posts.
    Interactions { month: u64 },
}

impl LogBucket {
    /// Places content by its expiry (ADR-0016): ephemeral content into the
    /// expiry window of the chip covering its remaining lifetime at `now`,
    /// permanent content into its post month.
    pub fn place(expires_at: Option<u64>, created_at: u64, now: u64) -> Self {
        match expires_at {
            Some(expires_at) => {
                let chip = LifetimeChip::for_remaining(expires_at.saturating_sub(now));
                Self::Expiry {
                    chip,
                    window: expires_at / chip.secs(),
                }
            }
            None => Self::PermanentMonth {
                month: created_at / MONTH_SECS,
            },
        }
    }

    /// Places a reaction or comment by its own creation month (see
    /// [`Self::Interactions`]).
    pub fn place_reaction(created_at: u64) -> Self {
        Self::Interactions {
            month: created_at / MONTH_SECS,
        }
    }

    /// The registry key this bucket allocates its log id under. Local
    /// authoring state only — never leaves the device (the wire carries the
    /// opaque log id, which reveals nothing about expiry).
    pub fn context_key(&self) -> String {
        match self {
            Self::Expiry { chip, window } => format!("bucket/{}/{window}", chip.label()),
            Self::PermanentMonth { month } => format!("bucket/perm/{month}"),
            Self::Interactions { month } => format!("react/{month}"),
        }
    }

    /// Whether GC may drop this whole bucket log the moment `now` passes its
    /// window, without inspecting individual payloads. True only for
    /// [`Self::Expiry`]: every post in an expiry window provably expires by the
    /// window's upper bound. Permanent and interaction buckets are never
    /// window-dropped (their contents leave individually / reactively).
    pub fn window_end(&self) -> Option<u64> {
        match self {
            Self::Expiry { chip, window } => Some((window + 1) * chip.secs()),
            Self::PermanentMonth { .. } | Self::Interactions { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_coarsen_up_to_the_next_rung() {
        assert_eq!(LifetimeChip::for_remaining(0), LifetimeChip::SixHours);
        assert_eq!(
            LifetimeChip::for_remaining(6 * 3600),
            LifetimeChip::SixHours
        );
        assert_eq!(
            LifetimeChip::for_remaining(6 * 3600 + 1),
            LifetimeChip::OneDay
        );
        assert_eq!(
            LifetimeChip::for_remaining(3 * 24 * 3600),
            LifetimeChip::OneWeek
        );
        // Beyond the ladder saturates at the coarsest tier.
        assert_eq!(
            LifetimeChip::for_remaining(10 * 365 * 24 * 3600),
            LifetimeChip::OneYear
        );
    }

    #[test]
    fn same_window_shares_a_bucket_and_adjacent_windows_do_not() {
        let now = 1_000_000;
        let chip = LifetimeChip::SixHours.secs();
        let window_start = ((now + 3600) / chip) * chip;
        let a = LogBucket::place(Some(window_start + 10), 0, now);
        let b = LogBucket::place(Some(window_start + chip - 1), 0, now);
        let c = LogBucket::place(Some(window_start + chip + 1), 0, now);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn rehomed_content_is_placed_by_remaining_time_not_age() {
        // A post created a year ago, re-homed with one day left to live,
        // belongs in a 1d bucket — not a 1y bucket.
        let now = 40_000_000;
        let created_at = now - 365 * 24 * 3600;
        let expires_at = now + 24 * 3600;
        match LogBucket::place(Some(expires_at), created_at, now) {
            LogBucket::Expiry { chip, .. } => assert_eq!(chip, LifetimeChip::OneDay),
            other => panic!("expected an expiry bucket, got {other:?}"),
        }
    }

    #[test]
    fn permanent_posts_bucket_by_post_month() {
        let a = LogBucket::place(None, 10 * MONTH_SECS + 1, 99_999_999);
        let b = LogBucket::place(None, 10 * MONTH_SECS + MONTH_SECS - 1, 0);
        let c = LogBucket::place(None, 11 * MONTH_SECS, 0);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.context_key(), "bucket/perm/10");
    }

    #[test]
    fn context_keys_are_distinct_across_tiers() {
        let day = LogBucket::Expiry {
            chip: LifetimeChip::OneDay,
            window: 7,
        };
        let week = LogBucket::Expiry {
            chip: LifetimeChip::OneWeek,
            window: 7,
        };
        assert_ne!(day.context_key(), week.context_key());
        assert_eq!(day.context_key(), "bucket/24h/7");
    }
}
