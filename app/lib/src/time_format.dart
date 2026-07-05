/// Lifetime formatting for the river's countdown pills. Port of the core's
/// former `time_format.rs`; keep thresholds in sync with the design.
library;

/// How urgently a post's remaining lifetime should read.
enum UrgencyTier {
  /// Permanent — no lifetime at all.
  settled,
  normal,

  /// Less than six hours left: the warm/amber treatment.
  warm,

  /// Less than one hour left.
  critical,
}

const int _warmThresholdSecs = 6 * 3600;
const int _criticalThresholdSecs = 3600;

typedef RemainingLabel = ({String label, UrgencyTier tier});

/// Formats the remaining lifetime of a post ("34h", "4h12m", "58m", "<1m")
/// with its urgency tier. `expiresAt <= now` renders as "ebbing away".
RemainingLabel formatRemaining(int now, int expiresAt) {
  final remaining = expiresAt - now;
  if (remaining <= 0) {
    return (label: 'ebbing away', tier: UrgencyTier.critical);
  }

  final tier = remaining < _criticalThresholdSecs
      ? UrgencyTier.critical
      : remaining < _warmThresholdSecs
          ? UrgencyTier.warm
          : UrgencyTier.normal;

  final String label;
  if (remaining >= 48 * 3600) {
    label = '${remaining ~/ (24 * 3600)}d';
  } else if (remaining >= 10 * 3600) {
    label = '${remaining ~/ 3600}h';
  } else if (remaining >= 3600) {
    final minutes = (remaining % 3600) ~/ 60;
    label = '${remaining ~/ 3600}h${minutes.toString().padLeft(2, '0')}m';
  } else if (remaining >= 60) {
    label = '${remaining ~/ 60}m';
  } else {
    label = '<1m';
  }

  return (label: label, tier: tier);
}

int nowUnixSecs() => DateTime.now().millisecondsSinceEpoch ~/ 1000;
