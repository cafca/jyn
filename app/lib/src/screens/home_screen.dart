import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../widgets/composer.dart';
import '../widgets/post_card.dart';
import 'diagnostics_screen.dart';
import 'friends_screen.dart';
import 'profile_screen.dart';
import 'settings_screen.dart';

/// The river: composer on top, posts flowing down, ghost doors at the end.
class HomeScreen extends ConsumerWidget {
  const HomeScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final posts = ref.watch(riverPostsProvider);
    final ghosts = ref.watch(ghostsProvider);
    final pendingCount = ref.watch(pendingRequestsProvider).length;

    return Scaffold(
      appBar: AppBar(
        title: const Text('jyn'),
        actions: [
          IconButton(
            tooltip: 'friends',
            onPressed: () => _push(context, const FriendsScreen()),
            icon: Badge(
              isLabelVisible: pendingCount > 0,
              label: Text('$pendingCount'),
              child: const Icon(Icons.group_outlined),
            ),
          ),
          IconButton(
            tooltip: 'profile',
            onPressed: () => _push(context, const ProfileScreen()),
            icon: const Icon(Icons.person_outline),
          ),
          IconButton(
            tooltip: 'diagnostics',
            onPressed: () => _push(context, const DiagnosticsScreen()),
            icon: const Icon(Icons.monitor_heart_outlined),
          ),
          IconButton(
            tooltip: 'settings',
            onPressed: () => _push(context, const SettingsScreen()),
            icon: const Icon(Icons.settings_outlined),
          ),
        ],
      ),
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 640),
          child: ListView(
            padding: const EdgeInsets.all(12),
            children: [
              const Composer(),
              const SizedBox(height: 12),
              if (posts.isEmpty)
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 48),
                  child: Center(
                    child: Text(
                      'the river is quiet',
                      style: Theme.of(context).textTheme.bodyLarge?.copyWith(
                            color: Theme.of(context).colorScheme.outline,
                          ),
                    ),
                  ),
                ),
              for (final post in posts)
                Padding(
                  padding: const EdgeInsets.only(bottom: 12),
                  child: PostCard(post: post),
                ),
              for (final ghost in ghosts)
                Padding(
                  padding: const EdgeInsets.only(bottom: 12),
                  child: _GhostDoor(
                    carrier: ghost.carrierDisplayName,
                    authorProfileId: ghost.authorProfileId,
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }

  void _push(BuildContext context, Widget screen) {
    Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => screen));
  }
}

/// A friend's heart on a stranger's post: a greyed-out door, not content.
class _GhostDoor extends StatelessWidget {
  const _GhostDoor({required this.carrier, required this.authorProfileId});

  final String carrier;
  final String authorProfileId;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Card(
      color: scheme.surfaceContainerHighest.withValues(alpha: 0.5),
      child: ListTile(
        leading: Icon(Icons.door_front_door_outlined, color: scheme.outline),
        title: Text(
          '$carrier hearted a post by ${shortId(authorProfileId)}…',
          style: TextStyle(color: scheme.outline),
        ),
        subtitle: Text(
          'not a friend yet — knock?',
          style: TextStyle(color: scheme.outline),
        ),
        trailing: OutlinedButton(
          onPressed: () => runGuarded(
            context,
            () => requestFriendshipById(profileId: authorProfileId),
          ),
          child: const Text('request'),
        ),
      ),
    );
  }
}
