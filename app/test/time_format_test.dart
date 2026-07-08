import 'package:flutter_test/flutter_test.dart';
import 'package:jyn/src/time_format.dart';

void main() {
  test('remaining durations format compactly', () {
    expect(formatRemaining(3 * 24 * 3600), '3d');
    expect(formatRemaining(34 * 3600), '34h');
    expect(formatRemaining(4 * 3600 + 12 * 60), '4h12m');
    expect(formatRemaining(58 * 60), '58m');
    expect(formatRemaining(30), '<1m');
  });

  test('urgency is relative to the original lifetime', () {
    // A fresh 6h post is normal, not warm — urgency tracks the post's own
    // scale, not wall-clock hours.
    final fresh = lifetimeState(now: 0, createdAt: 0, expiresAt: 6 * 3600);
    expect(fresh.tier, UrgencyTier.normal);
    expect(fresh.fraction, 1.0);

    // 24h post with 4h left: final fifth → warm/amber.
    final draining = lifetimeState(
      now: 20 * 3600,
      createdAt: 0,
      expiresAt: 24 * 3600,
    );
    expect(draining.tier, UrgencyTier.warm);
    expect(draining.label, '4h00m');

    // 24h post with 1h left: final twentieth → critical.
    final critical = lifetimeState(
      now: 23 * 3600,
      createdAt: 0,
      expiresAt: 24 * 3600,
    );
    expect(critical.tier, UrgencyTier.critical);

    // A year-long post at 4h remaining is also critical — relatively.
    final yearEnd = lifetimeState(
      now: 365 * 24 * 3600 - 4 * 3600,
      createdAt: 0,
      expiresAt: 365 * 24 * 3600,
    );
    expect(yearEnd.tier, UrgencyTier.critical);
  });

  test('an expired post reads as ebbing away at fraction zero', () {
    final gone = lifetimeState(now: 100, createdAt: 0, expiresAt: 100);
    expect(gone.label, 'ebbing away');
    expect(gone.tier, UrgencyTier.critical);
    expect(gone.fraction, 0);
  });

  test('ages format compactly', () {
    expect(formatAge(100, 90), 'now');
    expect(formatAge(10 * 60, 0), '10m');
    expect(formatAge(2 * 3600, 0), '2h');
    expect(formatAge(3 * 24 * 3600, 0), '3d');
  });
}
