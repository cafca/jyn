import 'package:flutter/material.dart' hide Visibility;

import '../actions.dart';
import '../rust/api/commands.dart';
import '../rust/profile.dart';

/// The first-hour flow: pick a name, learn the one rule (lifetime is the
/// only thing separating posts), step into the river.
class OnboardingScreen extends StatefulWidget {
  const OnboardingScreen({super.key, required this.profile});

  final UserProfile profile;

  @override
  State<OnboardingScreen> createState() => _OnboardingScreenState();
}

class _OnboardingScreenState extends State<OnboardingScreen> {
  late final TextEditingController _name = TextEditingController(
    text: widget.profile.displayName,
  );
  bool _busy = false;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _enter() async {
    setState(() => _busy = true);
    await runGuarded(context, () async {
      await updateProfile(
        displayName: _name.text.trim(),
        bio: widget.profile.bio,
        defaultVisibility: widget.profile.defaultVisibility,
        defaultLifetimeSecs: widget.profile.defaultLifetimeSecs,
        markOnboarded: true,
      );
    });
    if (mounted) setState(() => _busy = false);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Scaffold(
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 420),
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('jyn', style: theme.textTheme.displaySmall),
                const SizedBox(height: 8),
                Text(
                  'One river of posts, shared with friends you choose. '
                  'Ephemeral posts drain away; settled ones stay. '
                  'What should your friends call you?',
                  style: theme.textTheme.bodyLarge,
                ),
                const SizedBox(height: 24),
                TextField(
                  controller: _name,
                  autofocus: true,
                  decoration: const InputDecoration(
                    labelText: 'display name',
                    border: OutlineInputBorder(),
                  ),
                  onChanged: (_) => setState(() {}),
                  onSubmitted: (_) => _enter(),
                ),
                const SizedBox(height: 16),
                FilledButton(
                  onPressed: _busy || _name.text.trim().isEmpty ? null : _enter,
                  child: const Text('step into the river'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
