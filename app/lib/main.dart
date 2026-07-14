import 'dart:io';

import 'package:auto_updater/auto_updater.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show ExternalLibrary, ExternalLibraryLoaderConfig, loadExternalLibrary;
import 'package:media_kit/media_kit.dart';

import 'src/providers.dart';
import 'src/rust/api/lifecycle.dart';
import 'src/rust/frb_generated.dart';
import 'src/screens/home_screen.dart';
import 'src/screens/onboarding_screen.dart';
import 'src/screens/restore_gate.dart';
import 'src/shot/shot.dart';
import 'src/theme/tokens.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  // media_kit's single media engine (ADR 0018); must run before any Player is
  // constructed for voice notes or video.
  MediaKit.ensureInitialized();
  // Screenshot harness: boot a named screen on fixture data, capture a
  // PNG, exit. Never reachable without JYN_SHOT (see src/shot/shot.dart).
  final shot = shotScreen();
  if (shot != null) {
    await runShot(shot);
    return;
  }
  await RustLib.init(externalLibrary: await _loadCore());
  await _initAutoUpdater();
  // Restore is only possible before the node opens its stores, so a fresh
  // machine gets one chance to bring a backup in first.
  if (await isFreshInstall()) {
    await runRestoreGate();
  }
  await startNode();
  runApp(const ProviderScope(child: JynApp()));
}

/// Sparkle appcast, served from GitHub Pages and refreshed on every release.
const _updateFeedUrl = 'https://cafca.github.io/jyn/appcast.xml';

/// Bridges the native "Check for Updates…" menu item (see MainFlutterWindow)
/// to the Dart-side updater.
const _updaterChannel = MethodChannel('land.jyn.jyn/updater');

/// Wires up automatic (on-launch + daily) update checks and the manual menu
/// item. Only macOS ships an updater today; other desktops arrive with their
/// ports, and mobile updates through the app stores.
Future<void> _initAutoUpdater() async {
  if (!Platform.isMacOS) return;
  await autoUpdater.setFeedURL(_updateFeedUrl);
  await autoUpdater.setScheduledCheckInterval(86400);
  _updaterChannel.setMethodCallHandler((call) async {
    if (call.method == 'checkForUpdates') {
      await autoUpdater.checkForUpdates();
    }
    return null;
  });
}

/// On Apple platforms the Rust symbols are linked into the pod's framework,
/// which is named after the pub package (rust_lib_jyn) — not the crate name
/// the generated loader assumes. Elsewhere the default (libjyn) is correct.
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

class JynApp extends StatefulWidget {
  const JynApp({super.key});

  @override
  State<JynApp> createState() => _JynAppState();
}

class _JynAppState extends State<JynApp> with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    // Desktop notifications only fire while the app is unfocused.
    setAppFocused(focused: state == AppLifecycleState.resumed);
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'jyn',
      // Light-only: the design specifies a single near-white palette.
      theme: jynTheme(),
      home: const _Root(),
    );
  }
}

class _Root extends ConsumerWidget {
  const _Root();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Surface background errors (user-action failures throw at call sites).
    ref.listen(backgroundErrorsProvider, (previous, next) {
      if (next.isNotEmpty && next.length > (previous?.length ?? 0)) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(next.last)));
      }
    });

    final profile = ref.watch(profileProvider);
    if (profile == null) {
      return const Scaffold(body: Center(child: CircularProgressIndicator()));
    }
    if (!profile.onboarded) {
      return OnboardingScreen(profile: profile);
    }
    return const HomeScreen();
  }
}
