/// Lifetime made visible and honest: the countdown ring overlaid on media,
/// the inline "ebbs in …" pill for posts without visual media, and the
/// `◆ settled` chip for permanent posts. Rings tick on the shared 1 Hz
/// clock and turn amber over a post's final stretch (relative urgency —
/// see time_format.dart).
library;

import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../providers.dart';
import '../theme/tokens.dart';
import '../time_format.dart';

/// Top-right-of-media indicator: a 46px countdown ring for ebbing posts,
/// a still `◆ settled` chip for permanent ones.
class MediaLifetimeOverlay extends ConsumerWidget {
  const MediaLifetimeOverlay({
    super.key,
    required this.createdAt,
    required this.expiresAt,
  });

  final int createdAt;
  final int? expiresAt;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final expiresAt = this.expiresAt;
    if (expiresAt == null) {
      // On media the chip sits on a translucent light ground for legibility.
      return const SettledChip(translucent: true);
    }
    ref.watch(clockProvider);
    final state = lifetimeState(
      now: nowUnixSecs(),
      createdAt: createdAt,
      expiresAt: expiresAt,
    );
    final arc = state.tier == UrgencyTier.normal
        ? JynColors.ringTeal
        : JynColors.ringAmber;
    return SizedBox(
      width: 46,
      height: 46,
      child: CustomPaint(
        painter: _RingPainter(
          fraction: state.fraction,
          arcColor: arc,
          trackColor: JynColors.ringTrack,
          strokeWidth: 2.3,
          scrimColor: JynColors.ringScrim,
        ),
        child: Center(
          child: Text(
            state.label,
            style: const TextStyle(
              fontFamily: JynType.mono,
              fontSize: 10,
              color: JynColors.onMedia,
              height: 1.0,
            ),
          ),
        ),
      ),
    );
  }
}

/// Author-row lifetime for posts without photo/video media: an
/// "ebbs in 34h" pill with a tiny progress ring (teal, going amber as the
/// post drains), or the settled chip for permanent posts.
class LifetimePill extends ConsumerWidget {
  const LifetimePill({
    super.key,
    required this.createdAt,
    required this.expiresAt,
  });

  final int createdAt;
  final int? expiresAt;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final expiresAt = this.expiresAt;
    if (expiresAt == null) return const SettledChip();
    ref.watch(clockProvider);
    final state = lifetimeState(
      now: nowUnixSecs(),
      createdAt: createdAt,
      expiresAt: expiresAt,
    );
    final draining = state.tier != UrgencyTier.normal;
    final background = draining ? JynColors.drainingPillBg : JynColors.chipTint;
    final foreground = draining ? JynColors.drainingPillText : JynColors.mid;
    final ring = draining ? JynColors.ringAmberDeep : JynColors.ringTeal;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
      decoration: BoxDecoration(
        color: background,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          SizedBox(
            width: 11,
            height: 11,
            child: CustomPaint(
              painter: _RingPainter(
                fraction: state.fraction,
                arcColor: ring,
                trackColor: foreground.withValues(alpha: 0.25),
                strokeWidth: 2,
              ),
            ),
          ),
          const SizedBox(width: 5),
          Text(
            'ebbs in ${state.label}',
            style: TextStyle(
              fontFamily: JynType.mono,
              fontSize: 10.5,
              color: foreground,
              height: 1.0,
            ),
          ),
        ],
      ),
    );
  }
}

/// `◆ settled` — permanence as a still chip, never a ring.
class SettledChip extends StatelessWidget {
  const SettledChip({super.key, this.translucent = false, this.suffix});

  /// True when the chip sits on media rather than the near-white ground.
  final bool translucent;

  /// Optional trailing text, e.g. "· permanent" on the own-stream card.
  final String? suffix;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
      decoration: BoxDecoration(
        color: translucent ? const Color(0xD9FFFFFF) : JynColors.field,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Text(
        suffix == null ? '◆ settled' : '◆ settled $suffix',
        style: const TextStyle(
          fontFamily: JynType.mono,
          fontSize: 10.5,
          color: JynColors.slate,
          height: 1.0,
        ),
      ),
    );
  }
}

/// Scrim disc + track ring + progress arc, arc starting at 12 o'clock.
class _RingPainter extends CustomPainter {
  const _RingPainter({
    required this.fraction,
    required this.arcColor,
    required this.trackColor,
    required this.strokeWidth,
    this.scrimColor,
  });

  final double fraction;
  final Color arcColor;
  final Color trackColor;
  final double strokeWidth;
  final Color? scrimColor;

  @override
  void paint(Canvas canvas, Size size) {
    final center = Offset(size.width / 2, size.height / 2);
    final radius = size.width / 2 - strokeWidth;
    if (scrimColor != null) {
      canvas.drawCircle(center, size.width / 2, Paint()..color = scrimColor!);
    }
    final track = Paint()
      ..color = trackColor
      ..style = PaintingStyle.stroke
      ..strokeWidth = strokeWidth;
    canvas.drawCircle(center, radius, track);
    final arc = Paint()
      ..color = arcColor
      ..style = PaintingStyle.stroke
      ..strokeWidth = strokeWidth
      ..strokeCap = StrokeCap.round;
    canvas.drawArc(
      Rect.fromCircle(center: center, radius: radius),
      -math.pi / 2,
      2 * math.pi * fraction.clamp(0.0, 1.0),
      false,
      arc,
    );
  }

  @override
  bool shouldRepaint(_RingPainter oldDelegate) =>
      oldDelegate.fraction != fraction ||
      oldDelegate.arcColor != arcColor ||
      oldDelegate.trackColor != trackColor ||
      oldDelegate.scrimColor != scrimColor;
}
