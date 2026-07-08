/// Lifetime formatting for the river's countdown rings and pills.
///
/// Urgency is *relative* to a post's original lifetime (the design's ring
/// fraction is `remaining / originalLifetime`): the amber "draining"
/// treatment kicks in over the final fifth, critical over the final
/// twentieth — a 6h post and a 1-year post both drain over their own last
/// stretch rather than at a fixed wall-clock threshold.
library;

/// How urgently a post's remaining lifetime should read.
enum UrgencyTier {
  /// Permanent — no lifetime at all.
  settled,
  normal,

  /// The final fifth of the post's lifetime: the amber treatment.
  warm,

  /// The final twentieth (or already expired).
  critical,
}

const double _warmFraction = 0.20;
const double _criticalFraction = 0.05;

typedef LifetimeState = ({String label, UrgencyTier tier, double fraction});

/// Remaining-lifetime label, urgency tier, and ring fraction (0..1) for an
/// ebbing post. `expiresAt <= now` renders as "ebbing away" at fraction 0.
LifetimeState lifetimeState({
  required int now,
  required int createdAt,
  required int expiresAt,
}) {
  final remaining = expiresAt - now;
  if (remaining <= 0) {
    return (label: 'ebbing away', tier: UrgencyTier.critical, fraction: 0);
  }
  final original = expiresAt - createdAt;
  final fraction = original > 0 ? (remaining / original).clamp(0.0, 1.0) : 0.0;
  final tier = fraction <= _criticalFraction
      ? UrgencyTier.critical
      : fraction <= _warmFraction
      ? UrgencyTier.warm
      : UrgencyTier.normal;
  return (label: formatRemaining(remaining), tier: tier, fraction: fraction);
}

/// Formats a remaining duration in seconds: "3d", "34h", "4h12m", "58m",
/// "<1m".
String formatRemaining(int remaining) {
  if (remaining >= 48 * 3600) return '${remaining ~/ (24 * 3600)}d';
  if (remaining >= 10 * 3600) return '${remaining ~/ 3600}h';
  if (remaining >= 3600) {
    final minutes = (remaining % 3600) ~/ 60;
    return '${remaining ~/ 3600}h${minutes.toString().padLeft(2, '0')}m';
  }
  if (remaining >= 60) return '${remaining ~/ 60}m';
  return '<1m';
}

/// Compact age for author rows: "now", "5m", "2h", "3d".
String formatAge(int now, int createdAt) {
  final age = now - createdAt;
  if (age < 60) return 'now';
  if (age < 3600) return '${age ~/ 60}m';
  if (age < 48 * 3600) return '${age ~/ 3600}h';
  return '${age ~/ (24 * 3600)}d';
}

int nowUnixSecs() => DateTime.now().millisecondsSinceEpoch ~/ 1000;
