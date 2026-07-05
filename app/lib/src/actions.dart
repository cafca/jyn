/// Helpers shared by every screen that fires a command.
library;

import 'package:flutter/material.dart' hide Visibility;

import 'rust/domain.dart';

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

/// Composer lifetime presets (label, seconds; null = permanent). Mirrors the
/// old interface's options.
const lifetimeOptions = <(String, int?)>[
  ('1h', 3600),
  ('12h', 12 * 3600),
  ('36h', 36 * 3600),
  ('3d', 3 * 24 * 3600),
  ('1w', 7 * 24 * 3600),
  ('settled', null),
];

String visibilityLabel(Visibility visibility) => switch (visibility) {
  Visibility.friends => '◑ friends',
  Visibility.circles => '◑ circles',
  Visibility.public => '◉ public',
  Visibility.private => '◐ only you',
};

/// Visibilities offered by the composer (all of them) — profile defaults
/// exclude private, matching the core's validation.
const composerVisibilities = Visibility.values;
const defaultableVisibilities = [
  Visibility.friends,
  Visibility.circles,
  Visibility.public,
];

String shortId(String profileId) =>
    profileId.length <= 8 ? profileId : profileId.substring(0, 8);
