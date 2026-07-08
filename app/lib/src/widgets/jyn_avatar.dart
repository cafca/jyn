import 'package:flutter/material.dart';

import '../theme/tokens.dart';

/// A gradient initials circle. The gradient derives deterministically from
/// the profile id so a person renders identically everywhere (and on every
/// peer); the local user gets the brand's signature teal.
class JynAvatar extends StatelessWidget {
  const JynAvatar({
    super.key,
    required this.profileId,
    required this.displayName,
    this.size = 36,
    this.isSelf = false,
    this.onTap,
  });

  final String profileId;
  final String displayName;
  final double size;
  final bool isSelf;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final colors = isSelf
        ? JynColors.selfGradient
        : gradientForProfile(profileId);
    final avatar = Container(
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        gradient: LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight, // ≈ the mock's 140deg
          colors: colors,
        ),
      ),
      child: Text(
        initialsForName(displayName),
        style: TextStyle(
          color: Colors.white,
          fontSize: size * 0.34,
          fontWeight: FontWeight.w600,
          height: 1.0,
        ),
      ),
    );
    if (onTap == null) return avatar;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(onTap: onTap, child: avatar),
    );
  }
}

/// Light-over-dark two-stop gradient in a hue picked by hashing the
/// profile id — stable across sessions and peers.
List<Color> gradientForProfile(String profileId) {
  final hue = (_fnv1a(profileId) % 360).toDouble();
  return [
    HSLColor.fromAHSL(1, hue, 0.62, 0.76).toColor(),
    HSLColor.fromAHSL(1, hue, 0.45, 0.48).toColor(),
  ];
}

/// Up to two initials: first letters of the first two words, else the
/// first two characters, else "?" for an empty name.
String initialsForName(String displayName) {
  final words = displayName
      .trim()
      .split(RegExp(r'\s+'))
      .where((w) => w.isNotEmpty)
      .toList();
  if (words.isEmpty) return '?';
  if (words.length == 1) {
    final word = words.first;
    return word.substring(0, word.length.clamp(0, 2)).toUpperCase();
  }
  return (words[0][0] + words[1][0]).toUpperCase();
}

/// FNV-1a over UTF-16 code units; cheap and stable.
int _fnv1a(String input) {
  var hash = 0x811c9dc5;
  for (final unit in input.codeUnits) {
    hash ^= unit;
    hash = (hash * 0x01000193) & 0xFFFFFFFF;
  }
  return hash;
}
