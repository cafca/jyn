import 'package:flutter_test/flutter_test.dart';
import 'package:jyn/src/time_format.dart';

void main() {
  test('remaining lifetimes format with urgency tiers', () {
    expect(formatRemaining(0, 3 * 24 * 3600), (
      label: '3d',
      tier: UrgencyTier.normal,
    ));
    expect(formatRemaining(0, 34 * 3600), (
      label: '34h',
      tier: UrgencyTier.normal,
    ));
    expect(formatRemaining(0, 4 * 3600 + 12 * 60), (
      label: '4h12m',
      tier: UrgencyTier.warm,
    ));
    expect(formatRemaining(0, 58 * 60), (
      label: '58m',
      tier: UrgencyTier.critical,
    ));
    expect(formatRemaining(0, 30), (label: '<1m', tier: UrgencyTier.critical));
    expect(formatRemaining(100, 100), (
      label: 'ebbing away',
      tier: UrgencyTier.critical,
    ));
  });
}
