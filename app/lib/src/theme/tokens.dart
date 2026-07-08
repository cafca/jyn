/// Design tokens for the content-first (near-white) direction.
///
/// Every value here is lifted verbatim from the home-river design handoff
/// (`design_handoff_home_river/README.md`, "Design Tokens"). Screens build
/// from these tokens with primitive widgets rather than Material component
/// defaults; the Material [ThemeData] in [jynTheme] only covers the shell
/// (navigation, overlays, text editing) and the out-of-scope screens that
/// keep their Material form controls.
library;

import 'package:flutter/material.dart';

/// Colors — content-first direction.
abstract final class JynColors {
  // Window / surface.
  static const body = Color(0xFFFAF9F6);
  static const titlebar = Color(0xFFF3F2EE);
  static const cardGreen = Color(0xFFF1F5F2); // audio player card
  static const cardGrey = Color(0xFFF2F3F1); // unselected chips
  static const cardSettled = Color(0xFFF5F6F3); // settled text card
  static const field = Color(0xFFEEF0ED); // search field, quiet chips

  // Hairlines.
  static const hairline = Color(0x12000000); // rgba(0,0,0,.07)
  static const hairlineFaint = Color(0x0E000000); // rgba(0,0,0,.055)

  // Brand green.
  static const ink = Color(0xFF0B3B2E); // wordmark / strongest text
  static const leaf = Color(0xFF0B5C46); // brand mark + primary button
  static const mid = Color(0xFF2F6A58);
  static const chipTint = Color(0xFFEEF4F0); // reach pill bg
  static const chipSelected = Color(0xFFD7ECE4); // selected lifetime chip bg
  static const chipSelectedBorder = Color(0xFF7FD6BE);
  static const chipOutline = Color(0xFFBCDCCF); // outline Add button border
  static const sheetCardTop = Color(0xFFEEF7F2); // YOUR CODE gradient
  static const sheetCardBottom = Color(0xFFE2F0E9);
  static const sheetCardBorder = Color(0xFFCBE6DB);

  // Lifetime ring.
  static const ringTeal = Color(0xFF3FD0A6);
  static const ringAmber = Color(0xFFF0B56A);
  static const ringAmberDeep = Color(0xFFE0983F);
  static const ringTrack = Color(0x52FFFFFF); // rgba(255,255,255,.32)
  static const ringScrim = Color(0x6B08100D); // rgba(8,16,13,.42)

  // Amber "draining" treatment.
  static const drainingBg = Color(0xFFFAF3EA);
  static const drainingBorder = Color(0x80C98C4A); // rgba(201,140,74,.5)
  static const drainingPillBg = Color(0xFFF6E3CD);
  static const drainingPillText = Color(0xFF8A4A12);
  static const drainingBody = Color(0xFF4A3F30);

  // Hearts / friend requests.
  static const heart = Color(0xFFE0648F);
  static const requestBg = Color(0xFFFBEEF4);
  static const requestBorder = Color(0xFFF3C9DD);
  static const requestName = Color(0xFF8A2F5A);
  static const requestText = Color(0xFFA06883);
  static const accept = Color(0xFFC14D84);
  static const ignoreBg = Color(0xFFF6DBE7);

  // Text.
  static const text = Color(0xFF1B241F);
  static const textSoft = Color(0xFF28322C);
  static const secondary = Color(0xFF8A978F);
  static const muted = Color(0xFF9AA79F);
  static const slate = Color(0xFF5A6A62); // settled chip text, attach icon
  static const onMedia = Colors.white;
  static const onMediaSoft = Color(0xD9FFFFFF); // rgba(255,255,255,.85)

  // Audio waveform fade, left → right.
  static const waveform = [
    Color(0xFF0B5C46),
    Color(0xFF159C78),
    Color(0xFF7DC9B6),
    Color(0xFFB7DDD2),
  ];
  static const audioDuration = Color(0xFF2F6A58);

  // The self avatar's signature gradient; friends derive theirs from their
  // profile id (see JynAvatar).
  static const selfGradient = [Color(0xFF7FD6BE), Color(0xFF1C8F78)];

  // Floating composer surfaces.
  static const composerPillBg = Color(0xD1FFFFFF); // rgba(255,255,255,.82)
  static const composerCardBg = Color(0xE6FFFFFF); // rgba(255,255,255,.9)
}

/// Radii.
abstract final class JynRadii {
  static const media = 14.0;
  static const card = 16.0;
  static const sheet = 22.0; // sheets & expanded composer
  static const pill = 26.0; // floating composer pill
  static const chip = 9.0;
  static const button = 20.0; // cast button
  static const attach = 11.0; // rounded-square attach button
}

/// Shadows.
abstract final class JynShadows {
  static const floatingPill = [
    BoxShadow(
      color: Color(0x471E2D28), // rgba(30,45,40,.28)
      blurRadius: 30,
      spreadRadius: -8,
      offset: Offset(0, 12),
    ),
  ];
  static const expandedCard = [
    BoxShadow(
      color: Color(0x57121C18), // rgba(18,28,24,.34)
      blurRadius: 44,
      spreadRadius: -12,
      offset: Offset(0, 20),
    ),
  ];
  static const primaryButton = [
    BoxShadow(
      color: Color(0x990B5C46), // rgba(11,92,70,.6)
      blurRadius: 16,
      spreadRadius: -6,
      offset: Offset(0, 6),
    ),
  ];
}

/// Layout constants.
abstract final class JynLayout {
  /// The single centered content column, everywhere.
  static const column = 440.0;

  static const toolbarHeight = 56.0;

  /// Extra top padding on macOS so toolbar content clears the floating
  /// traffic lights (the native titlebar is transparent/full-size-content).
  static const titlebarInset = 34.0;
}

/// Text styles. UI/body stays the platform system sans (Material's macOS
/// typography); SpaceMono covers durations, timestamps, codes and small
/// ALL-CAPS labels.
abstract final class JynType {
  static const mono = 'SpaceMono';

  static const wordmark = TextStyle(
    fontSize: 23,
    fontWeight: FontWeight.w700,
    color: JynColors.ink,
    letterSpacing: -0.69, // -.03em
    height: 1.0,
  );

  static const name = TextStyle(
    fontSize: 14,
    fontWeight: FontWeight.w600,
    color: JynColors.text,
  );

  static const body = TextStyle(
    fontSize: 14,
    height: 1.5,
    color: JynColors.text,
  );

  static const meta = TextStyle(fontSize: 11.5, color: JynColors.secondary);

  static const metaMono = TextStyle(
    fontFamily: mono,
    fontSize: 11.5,
    color: JynColors.secondary,
    letterSpacing: 0.46, // .04em
  );

  /// Small ALL-CAPS mono label (e.g. "YOUR CODE").
  static const capsLabel = TextStyle(
    fontFamily: mono,
    fontSize: 10.5,
    color: JynColors.mid,
    letterSpacing: 1.47, // .14em
    fontWeight: FontWeight.w700,
  );

  static const shareCode = TextStyle(
    fontFamily: mono,
    fontSize: 17,
    color: JynColors.leaf,
    letterSpacing: 0.5,
  );
}

/// The Material shell theme: light-only, near-white, brand-green accents.
/// Designed surfaces render straight from tokens; this theme keeps the
/// Material components on the out-of-scope screens (settings, diagnostics)
/// on palette.
ThemeData jynTheme() {
  final scheme =
      ColorScheme.fromSeed(
        seedColor: JynColors.leaf,
        brightness: Brightness.light,
      ).copyWith(
        primary: JynColors.leaf,
        onPrimary: Colors.white,
        surface: JynColors.body,
        onSurface: JynColors.text,
        outline: JynColors.secondary,
      );
  return ThemeData(
    colorScheme: scheme,
    useMaterial3: true,
    scaffoldBackgroundColor: JynColors.body,
    visualDensity: VisualDensity.comfortable,
    splashFactory: NoSplash.splashFactory,
    highlightColor: Colors.transparent,
    hoverColor: JynColors.field,
    // Screens swap instantly — no push/pop animation.
    pageTransitionsTheme: PageTransitionsTheme(
      builders: {
        for (final platform in TargetPlatform.values)
          platform: const _InstantPageTransitionsBuilder(),
      },
    ),
    dividerTheme: const DividerThemeData(
      color: JynColors.hairline,
      thickness: 1,
      space: 1,
    ),
    tooltipTheme: _tooltipTheme(),
  );
}

TooltipThemeData _tooltipTheme() {
  return TooltipThemeData(
    waitDuration: const Duration(milliseconds: 350),
    decoration: BoxDecoration(
      color: JynColors.ink,
      borderRadius: BorderRadius.circular(8),
    ),
    textStyle: const TextStyle(fontSize: 12, color: Colors.white),
  );
}

/// No route animation at all: the new screen just appears.
class _InstantPageTransitionsBuilder extends PageTransitionsBuilder {
  const _InstantPageTransitionsBuilder();

  @override
  Widget buildTransitions<T>(
    PageRoute<T> route,
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
    Widget child,
  ) => child;
}
