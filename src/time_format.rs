//! Lifetime formatting for the river's countdown pills.

/// How urgently a post's remaining lifetime should read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UrgencyTier {
    /// Permanent — no lifetime at all.
    Settled,
    Normal,
    /// Less than six hours left: the warm/amber treatment.
    Warm,
    /// Less than one hour left.
    Critical,
}

const WARM_THRESHOLD_SECS: u64 = 6 * 3600;
const CRITICAL_THRESHOLD_SECS: u64 = 3600;

/// Formats the remaining lifetime of a post ("34h", "4h12m", "58m", "<1m")
/// with its urgency tier. `expires_at <= now` renders as "ebbing away".
pub(crate) fn format_remaining(now: u64, expires_at: u64) -> (String, UrgencyTier) {
    let Some(remaining) = expires_at.checked_sub(now).filter(|left| *left > 0) else {
        return ("ebbing away".to_owned(), UrgencyTier::Critical);
    };

    let tier = if remaining < CRITICAL_THRESHOLD_SECS {
        UrgencyTier::Critical
    } else if remaining < WARM_THRESHOLD_SECS {
        UrgencyTier::Warm
    } else {
        UrgencyTier::Normal
    };

    let label = if remaining >= 48 * 3600 {
        format!("{}d", remaining / (24 * 3600))
    } else if remaining >= 10 * 3600 {
        format!("{}h", remaining / 3600)
    } else if remaining >= 3600 {
        format!("{}h{:02}m", remaining / 3600, (remaining % 3600) / 60)
    } else if remaining >= 60 {
        format!("{}m", remaining / 60)
    } else {
        "<1m".to_owned()
    };

    (label, tier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_lifetimes_format_with_urgency_tiers() {
        assert_eq!(
            format_remaining(0, 3 * 24 * 3600),
            ("3d".to_owned(), UrgencyTier::Normal)
        );
        assert_eq!(
            format_remaining(0, 34 * 3600),
            ("34h".to_owned(), UrgencyTier::Normal)
        );
        assert_eq!(
            format_remaining(0, 4 * 3600 + 12 * 60),
            ("4h12m".to_owned(), UrgencyTier::Warm)
        );
        assert_eq!(
            format_remaining(0, 58 * 60),
            ("58m".to_owned(), UrgencyTier::Critical)
        );
        assert_eq!(
            format_remaining(0, 30),
            ("<1m".to_owned(), UrgencyTier::Critical)
        );
        assert_eq!(
            format_remaining(100, 100),
            ("ebbing away".to_owned(), UrgencyTier::Critical)
        );
    }
}
