/// Screenshot harness: boot the app on a named screen with fixture data,
/// render it, write a PNG, and exit — non-zero if the framework reported
/// any error while building/laying out the screen.
///
/// Trigger with `--dart-define=JYN_SHOT=<screen>` at build time, or the
/// `JYN_SHOT` environment variable at launch (one build, many runs):
///
///   JYN_DATA_DIR=/tmp/jyn-shot JYN_SHOT=home JYN_SHOT_OUT=/tmp/home.png \
///     ./build/macos/Build/Products/Debug/jyn.app/Contents/MacOS/jyn
///
/// Screens: onboarding · home · composer · profile · add_friend ·
/// edit_profile · settings · diagnostics.
library;

import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter/rendering.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show ExternalLibrary, ExternalLibraryLoaderConfig, loadExternalLibrary;

import '../providers.dart';
import '../rust/api/lifecycle.dart';
import '../rust/frb_generated.dart';
import '../screens/diagnostics_screen.dart';
import '../screens/home_screen.dart';
import '../screens/onboarding_screen.dart';
import '../screens/profile_screen.dart';
import '../screens/settings_screen.dart';
import '../theme/tokens.dart';
import '../time_format.dart';
import 'fixtures.dart';

/// The requested shot screen, or null for a normal launch.
String? shotScreen() {
  const fromDefine = String.fromEnvironment('JYN_SHOT');
  if (fromDefine.isNotEmpty) return fromDefine;
  final fromEnv = Platform.environment['JYN_SHOT'];
  return (fromEnv == null || fromEnv.isEmpty) ? null : fromEnv;
}

String _shotOutPath(String screen) {
  const fromDefine = String.fromEnvironment('JYN_SHOT_OUT');
  if (fromDefine.isNotEmpty) return fromDefine;
  return Platform.environment['JYN_SHOT_OUT'] ?? 'jyn-shot-$screen.png';
}

final _errors = <String>[];
final _boundaryKey = GlobalKey();

Future<void> runShot(String screen) async {
  // Never let a screenshot run touch the real store.
  if (Platform.environment['JYN_DATA_DIR'] == null) {
    stderr.writeln('JYN_SHOT refuses to run without JYN_DATA_DIR set.');
    exit(2);
  }

  FlutterError.onError = (details) {
    _errors.add(details.exceptionAsString());
    FlutterError.presentError(details);
  };
  ui.PlatformDispatcher.instance.onError = (error, stack) {
    _errors.add('$error');
    return true;
  };

  // The core still backs live calls (settings, friend code); the event
  // stream itself is replaced by fixtures below.
  await RustLib.init(externalLibrary: await _loadCore());
  await startNode();

  final now = nowUnixSecs();
  final photoPath = await _writeFixturePhoto();
  final audioPath =
      '${Directory.systemTemp.path}/jyn-shot-audio.wav'; // never played

  runApp(
    ProviderScope(
      overrides: [
        jynEventsProvider.overrideWithValue(
          Stream.fromIterable(
            shotEvents(now: now, photoPath: photoPath, audioPath: audioPath),
          ),
        ),
      ],
      child: RepaintBoundary(
        key: _boundaryKey,
        child: MaterialApp(
          title: 'jyn shot',
          debugShowCheckedModeBanner: false,
          theme: jynTheme(),
          home: _screenFor(screen, now),
        ),
      ),
    ),
  );

  // Let fixture events land, images decode, and any post-frame sheets open.
  await Future<void>.delayed(const Duration(milliseconds: 1800));
  final outPath = _shotOutPath(screen);
  try {
    final boundary =
        _boundaryKey.currentContext!.findRenderObject()!
            as RenderRepaintBoundary;
    final image = await boundary.toImage(pixelRatio: 2);
    final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
    await File(outPath).writeAsBytes(bytes!.buffer.asUint8List());
  } catch (error) {
    _errors.add('capture failed: $error');
  }

  if (_errors.isEmpty) {
    stdout.writeln('SHOT OK $screen -> $outPath');
    exit(0);
  }
  stderr.writeln('SHOT FAILED $screen:\n${_errors.join('\n')}');
  exit(1);
}

Widget _screenFor(String screen, int now) => switch (screen) {
  'onboarding' => OnboardingScreen(
    profile: shotProfile(now: now, onboarded: false),
  ),
  'home' => const HomeScreen(),
  'composer' => const HomeScreen(composerExpanded: true),
  'profile' => const ProfileScreen(),
  'add_friend' => const ProfileScreen(initialSheet: ProfileSheet.addFriend),
  'edit_profile' => const ProfileScreen(initialSheet: ProfileSheet.editProfile),
  'settings' => const SettingsScreen(),
  'diagnostics' => const DiagnosticsScreen(),
  _ => throw ArgumentError('unknown JYN_SHOT screen: $screen'),
};

/// Same loader as main.dart: on Apple platforms the Rust symbols live in
/// the pod's framework named after the pub package.
Future<ExternalLibrary?> _loadCore() async {
  if (!Platform.isMacOS && !Platform.isIOS) return null;
  return loadExternalLibrary(
    const ExternalLibraryLoaderConfig(
      stem: 'rust_lib_jyn',
      ioDirectory: '../core/target/release/',
      webPrefix: null,
    ),
  );
}

/// A quiet 800×1000 gradient stand-in for a real photo blob.
Future<String> _writeFixturePhoto() async {
  final recorder = ui.PictureRecorder();
  final canvas = Canvas(recorder);
  const size = Size(800, 1000);
  canvas.drawRect(
    Offset.zero & size,
    Paint()
      ..shader = ui.Gradient.linear(Offset.zero, const Offset(800, 1000), [
        const Color(0xFFCDD8CF),
        const Color(0xFF88A293),
      ]),
  );
  canvas.drawCircle(
    const Offset(560, 300),
    140,
    Paint()..color = const Color(0x55FFFFFF),
  );
  final image = await recorder.endRecording().toImage(800, 1000);
  final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
  final path = '${Directory.systemTemp.path}/jyn-shot-photo.png';
  await File(path).writeAsBytes(bytes!.buffer.asUint8List());
  return path;
}
