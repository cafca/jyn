import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../rust/api/lifecycle.dart' as rust;

/// Consented friendship, nothing else: your code, their code, pending
/// requests, and the circle itself.
class FriendsScreen extends ConsumerStatefulWidget {
  const FriendsScreen({super.key});

  @override
  ConsumerState<FriendsScreen> createState() => _FriendsScreenState();
}

class _FriendsScreenState extends ConsumerState<FriendsScreen> {
  final _codeInput = TextEditingController();

  @override
  void dispose() {
    _codeInput.dispose();
    super.dispose();
  }

  Future<void> _request() async {
    final code = _codeInput.text.trim();
    if (!code.startsWith('jyn-')) return;
    await runGuarded(context, () => requestFriendship(friendCode: code));
    _codeInput.clear();
    setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final friends = ref.watch(friendsProvider);
    final pending = ref.watch(pendingRequestsProvider);

    return Scaffold(
      appBar: AppBar(title: const Text('friends')),
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 640),
          child: ListView(
            padding: const EdgeInsets.all(16),
            children: [
              Text('your code', style: theme.textTheme.titleSmall),
              const SizedBox(height: 4),
              Text(
                'Hand it over any channel you trust.',
                style: theme.textTheme.bodySmall,
              ),
              const SizedBox(height: 8),
              FutureBuilder<String>(
                future: rust.myFriendCode(),
                builder: (context, snapshot) {
                  final code = snapshot.data;
                  return Row(
                    children: [
                      Expanded(
                        child: Text(
                          code == null
                              ? '…'
                              : '${code.substring(0, code.length.clamp(0, 40))}…',
                          style: theme.textTheme.bodySmall?.copyWith(
                            fontFamily: 'monospace',
                          ),
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      IconButton(
                        tooltip: 'copy full code',
                        onPressed: code == null
                            ? null
                            : () async {
                                await Clipboard.setData(
                                  ClipboardData(text: code),
                                );
                                if (context.mounted) {
                                  ScaffoldMessenger.of(context).showSnackBar(
                                    const SnackBar(
                                      content: Text('code copied'),
                                    ),
                                  );
                                }
                              },
                        icon: const Icon(Icons.copy, size: 18),
                      ),
                    ],
                  );
                },
              ),
              const Divider(height: 32),
              Text('add a friend', style: theme.textTheme.titleSmall),
              const SizedBox(height: 8),
              Row(
                children: [
                  Expanded(
                    child: TextField(
                      controller: _codeInput,
                      decoration: const InputDecoration(
                        hintText: 'paste a jyn- code…',
                        isDense: true,
                        border: OutlineInputBorder(),
                      ),
                      onChanged: (_) => setState(() {}),
                      onSubmitted: (_) => _request(),
                    ),
                  ),
                  const SizedBox(width: 8),
                  FilledButton(
                    onPressed: _codeInput.text.trim().startsWith('jyn-')
                        ? _request
                        : null,
                    child: const Text('＋ request'),
                  ),
                ],
              ),
              if (pending.isNotEmpty) ...[
                const Divider(height: 32),
                Text('knocking', style: theme.textTheme.titleSmall),
                for (final request in pending)
                  ListTile(
                    contentPadding: EdgeInsets.zero,
                    title: Text(request.requesterDisplayName),
                    subtitle: request.greeting != null
                        ? Text('“${request.greeting}”')
                        : null,
                    trailing: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        FilledButton(
                          onPressed: () => runGuarded(
                            context,
                            () => respondFriendship(
                              requesterProfileId: request.requesterProfileId,
                              accept: true,
                            ),
                          ),
                          child: const Text('accept'),
                        ),
                        const SizedBox(width: 8),
                        OutlinedButton(
                          onPressed: () => runGuarded(
                            context,
                            () => respondFriendship(
                              requesterProfileId: request.requesterProfileId,
                              accept: false,
                            ),
                          ),
                          child: const Text('decline'),
                        ),
                      ],
                    ),
                  ),
              ],
              const Divider(height: 32),
              Text('the circle', style: theme.textTheme.titleSmall),
              if (friends.isEmpty)
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 16),
                  child: Text(
                    'nobody yet — trade codes with a friend',
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: theme.colorScheme.outline,
                    ),
                  ),
                ),
              for (final friend in friends)
                ListTile(
                  contentPadding: EdgeInsets.zero,
                  leading: Icon(
                    friend.followsMeBack
                        ? Icons.handshake_outlined
                        : Icons.hourglass_top,
                    size: 20,
                  ),
                  title: Text(friend.displayName),
                  subtitle: Text(
                    friend.followsMeBack
                        ? shortId(friend.profileId)
                        : '${shortId(friend.profileId)} — awaiting their answer',
                  ),
                  trailing: IconButton(
                    tooltip: 'unfriend',
                    onPressed: () =>
                        _confirmUnfriend(friend.displayName, friend.profileId),
                    icon: const Icon(Icons.person_remove_outlined, size: 20),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }

  Future<void> _confirmUnfriend(String name, String profileId) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('unfriend $name?'),
        content: const Text(
          'Their river dries up for you, and yours for them.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('unfriend'),
          ),
        ],
      ),
    );
    if (confirmed == true && mounted) {
      await runGuarded(context, () => removeFriend(profileId: profileId));
    }
  }
}
