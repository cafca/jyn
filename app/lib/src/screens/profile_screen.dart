import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../rust/domain.dart';

/// The local profile: name, bio, and the composer defaults.
class ProfileScreen extends ConsumerStatefulWidget {
  const ProfileScreen({super.key});

  @override
  ConsumerState<ProfileScreen> createState() => _ProfileScreenState();
}

class _ProfileScreenState extends ConsumerState<ProfileScreen> {
  TextEditingController? _name;
  TextEditingController? _bio;
  Visibility? _defaultVisibility;
  int? _defaultLifetime;
  bool _lifetimeTouched = false;
  bool _saving = false;

  @override
  void dispose() {
    _name?.dispose();
    _bio?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final profile = ref.watch(profileProvider);
    if (profile == null) {
      return const Scaffold(body: Center(child: CircularProgressIndicator()));
    }

    final name =
        _name ??= TextEditingController(text: profile.displayName);
    final bio = _bio ??= TextEditingController(text: profile.bio);
    final visibility = _defaultVisibility ?? profile.defaultVisibility;
    final lifetime =
        _lifetimeTouched ? _defaultLifetime : profile.defaultLifetimeSecs;

    return Scaffold(
      appBar: AppBar(title: const Text('profile')),
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 640),
          child: ListView(
            padding: const EdgeInsets.all(16),
            children: [
              Text(
                shortId(profile.profileId),
                style: theme.textTheme.labelSmall
                    ?.copyWith(fontFamily: 'monospace'),
              ),
              const SizedBox(height: 16),
              TextField(
                controller: name,
                decoration: const InputDecoration(
                  labelText: 'display name',
                  border: OutlineInputBorder(),
                ),
              ),
              const SizedBox(height: 16),
              TextField(
                controller: bio,
                minLines: 2,
                maxLines: 4,
                decoration: const InputDecoration(
                  labelText: 'bio',
                  border: OutlineInputBorder(),
                ),
              ),
              const SizedBox(height: 24),
              Text('default visibility', style: theme.textTheme.titleSmall),
              const SizedBox(height: 8),
              SegmentedButton<Visibility>(
                segments: [
                  for (final option in defaultableVisibilities)
                    ButtonSegment(
                      value: option,
                      label: Text(visibilityLabel(option)),
                    ),
                ],
                selected: {visibility},
                onSelectionChanged: (selection) =>
                    setState(() => _defaultVisibility = selection.first),
              ),
              const SizedBox(height: 24),
              Text('default lifetime', style: theme.textTheme.titleSmall),
              const SizedBox(height: 8),
              Wrap(
                spacing: 4,
                children: [
                  for (final (label, secs) in lifetimeOptions)
                    ChoiceChip(
                      label: Text(label),
                      selected: lifetime == secs,
                      onSelected: (_) => setState(() {
                        _lifetimeTouched = true;
                        _defaultLifetime = secs;
                      }),
                    ),
                ],
              ),
              const SizedBox(height: 32),
              FilledButton(
                onPressed: _saving
                    ? null
                    : () async {
                        setState(() => _saving = true);
                        await runGuarded(context, () async {
                          await updateProfile(
                            displayName: name.text.trim(),
                            bio: bio.text.trim(),
                            defaultVisibility: visibility,
                            defaultLifetimeSecs: lifetime,
                            markOnboarded: false,
                          );
                        });
                        if (mounted) setState(() => _saving = false);
                      },
                child: const Text('save'),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
