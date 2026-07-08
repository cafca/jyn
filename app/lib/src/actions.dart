/// Helpers shared by every screen that fires a command.
library;

import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'providers.dart';
import 'rust/domain.dart';
import 'rust/state.dart';
import 'screens/profile_screen.dart';
import 'screens/user_profile_screen.dart';

/// Clicking any avatar or name lands on that person's profile — the local
/// user's own profile screen for self, a read view for everyone else.
void openUserProfile(
  BuildContext context,
  WidgetRef ref, {
  required String profileId,
  required String displayName,
}) {
  final isSelf = ref.read(profileProvider)?.profileId == profileId;
  Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => isSelf
          ? const ProfileScreen()
          : UserProfileScreen(profileId: profileId, displayName: displayName),
    ),
  );
}

/// Runs a user action; failures land in a snackbar instead of crashing.
Future<void> runGuarded(
  BuildContext context,
  Future<void> Function() action,
) async {
  try {
    await action();
  } catch (error) {
    if (context.mounted) {
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text(error.toString())));
    }
  }
}

/// The canonical lifetime scale (label, seconds; null = permanent) — the
/// design's five steps, used everywhere a lifetime is chosen.
const lifetimeOptions = <(String, int?)>[
  ('6h', 6 * 3600),
  ('24h', 24 * 3600),
  ('1 week', 7 * 24 * 3600),
  ('1 year', 365 * 24 * 3600),
  ('permanent', null),
];

/// The finite steps only — what "make ephemeral" offers.
const ephemeralLifetimeOptions = <(String, int)>[
  ('6h', 6 * 3600),
  ('24h', 24 * 3600),
  ('1 week', 7 * 24 * 3600),
  ('1 year', 365 * 24 * 3600),
];

/// Reach glyphs per the design; `◆` is reserved for the settled lifetime
/// chip and never marks reach.
String visibilityGlyph(Visibility visibility) => switch (visibility) {
  Visibility.circles => '◑',
  Visibility.friends => '◐',
  Visibility.public => '◉',
  Visibility.private => '●',
};

String visibilityName(Visibility visibility) => switch (visibility) {
  Visibility.circles => 'circles',
  Visibility.friends => 'friends',
  Visibility.public => 'public',
  Visibility.private => 'only you',
};

String visibilityLabel(Visibility visibility) =>
    '${visibilityGlyph(visibility)} ${visibilityName(visibility)}';

/// Visibilities offered by the composer (all of them).
const composerVisibilities = Visibility.values;

/// Named hearts, never a bare count: "Mira", "Mira and Soren",
/// "Mira, Soren and others". The full list stays a tap away.
String heartsSummary(List<RiverHeart> hearts, {int maxNames = 2}) {
  if (hearts.isEmpty) return '';
  final names = hearts.map((h) => h.hearterDisplayName).toList();
  if (names.length <= maxNames) {
    return names.length == 1
        ? names.single
        : '${names.sublist(0, names.length - 1).join(', ')} and ${names.last}';
  }
  return '${names.sublist(0, maxNames).join(', ')} and others';
}

String shortId(String profileId) =>
    profileId.length <= 8 ? profileId : profileId.substring(0, 8);
