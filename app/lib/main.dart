import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'src/providers.dart';
import 'src/rust/api/lifecycle.dart';
import 'src/rust/frb_generated.dart';
import 'src/screens/home_screen.dart';
import 'src/screens/onboarding_screen.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  await startNode();
  runApp(const ProviderScope(child: JynApp()));
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
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xFF2A9D8F)),
        useMaterial3: true,
        visualDensity: VisualDensity.comfortable,
      ),
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
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text(next.last)));
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
