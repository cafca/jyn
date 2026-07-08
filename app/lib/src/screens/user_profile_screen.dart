import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../theme/chrome.dart';
import '../theme/tokens.dart';
import '../widgets/jyn_avatar.dart';
import '../widgets/post_card.dart';

/// Another person's profile: identity, friendship state, and whatever of
/// their river we see. (The local user's own profile is ProfileScreen.)
class UserProfileScreen extends ConsumerWidget {
  const UserProfileScreen({
    super.key,
    required this.profileId,
    required this.displayName,
  });

  final String profileId;
  final String displayName;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final friends = ref.watch(friendsProvider);
    final posts = ref
        .watch(riverPostsProvider)
        .where((p) => p.authorProfileId == profileId)
        .toList();

    // Prefer the freshest display name from the friend list.
    var name = displayName;
    var isFriend = false;
    var mutual = false;
    for (final friend in friends) {
      if (friend.profileId == profileId) {
        name = friend.displayName;
        isFriend = true;
        mutual = friend.followsMeBack;
        break;
      }
    }

    return Scaffold(
      body: Column(
        children: [
          const JynTitlebarStrip(),
          const JynToolbar(showBack: true, title: 'Profile'),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.symmetric(vertical: 22),
              children: [
                JynColumnItem(
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      JynAvatar(
                        profileId: profileId,
                        displayName: name,
                        size: 70,
                      ),
                      const SizedBox(width: 16),
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            const SizedBox(height: 8),
                            Text(
                              name,
                              style: const TextStyle(
                                fontSize: 20,
                                fontWeight: FontWeight.w700,
                                color: JynColors.ink,
                              ),
                              overflow: TextOverflow.ellipsis,
                            ),
                            const SizedBox(height: 4),
                            Text(
                              !isFriend
                                  ? 'not in your circle'
                                  : mutual
                                  ? 'friend'
                                  : 'awaiting their answer',
                              style: JynType.meta,
                            ),
                          ],
                        ),
                      ),
                      if (isFriend)
                        MouseRegion(
                          cursor: SystemMouseCursors.click,
                          child: GestureDetector(
                            onTap: () => _confirmUnfriend(context, name),
                            child: Container(
                              padding: const EdgeInsets.symmetric(
                                horizontal: 10,
                                vertical: 4,
                              ),
                              decoration: BoxDecoration(
                                color: JynColors.field,
                                borderRadius: BorderRadius.circular(999),
                              ),
                              child: Text(
                                'unfriend',
                                style: JynType.body.copyWith(
                                  fontSize: 12,
                                  color: JynColors.slate,
                                ),
                              ),
                            ),
                          ),
                        ),
                    ],
                  ),
                ),
                const SizedBox(height: 22),
                const JynColumnItem(child: JynHairline(faint: true)),
                if (posts.isEmpty)
                  JynColumnItem(
                    child: Padding(
                      padding: const EdgeInsets.symmetric(vertical: 40),
                      child: Center(
                        child: Text(
                          'their river is quiet',
                          style: JynType.body.copyWith(color: JynColors.muted),
                        ),
                      ),
                    ),
                  ),
                for (final (index, post) in posts.indexed) ...[
                  if (index > 0)
                    const JynColumnItem(child: JynHairline(faint: true)),
                  JynColumnItem(child: PostCard(post: post)),
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }

  Future<void> _confirmUnfriend(BuildContext context, String name) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        backgroundColor: JynColors.body,
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
    if (confirmed == true && context.mounted) {
      await runGuarded(context, () => removeFriend(profileId: profileId));
    }
  }
}
