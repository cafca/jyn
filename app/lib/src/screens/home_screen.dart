import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../theme/chrome.dart';
import '../theme/tokens.dart';
import '../widgets/composer.dart';
import '../widgets/jyn_avatar.dart';
import '../widgets/post_card.dart';

/// The river (`5a`): a single immersive 440px column of posts on the
/// near-white ground, chrome pared back to wordmark, search and profile,
/// with the floating cast bar pinned bottom-center.
class HomeScreen extends ConsumerStatefulWidget {
  const HomeScreen({super.key, this.composerExpanded = false});

  /// Boot with the composer expanded (screenshot harness only).
  final bool composerExpanded;

  @override
  ConsumerState<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends ConsumerState<HomeScreen> {
  final _scroll = ScrollController();

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  void _scrollToTop() {
    if (!_scroll.hasClients) return;
    _scroll.animateTo(
      0,
      duration: const Duration(milliseconds: 350),
      curve: Curves.easeOutCubic,
    );
  }

  @override
  Widget build(BuildContext context) {
    final posts = ref.watch(riverPostsProvider);
    final ghosts = ref.watch(ghostsProvider);
    final profile = ref.watch(profileProvider);
    final pendingCount = ref.watch(pendingRequestsProvider).length;

    return Scaffold(
      body: Column(
        children: [
          const JynTitlebarStrip(),
          JynToolbar(
            onWordmarkTap: _scrollToTop,
            actions: [
              const JynSearchField(),
              const SizedBox(width: 14),
              Badge(
                isLabelVisible: pendingCount > 0,
                backgroundColor: JynColors.accept,
                smallSize: 8,
                child: JynAvatar(
                  profileId: profile?.profileId ?? '',
                  displayName: profile?.displayName ?? '',
                  size: 30,
                  isSelf: true,
                  onTap: profile == null
                      ? null
                      : () => openUserProfile(
                          context,
                          ref,
                          profileId: profile.profileId,
                          displayName: profile.displayName,
                        ),
                ),
              ),
            ],
          ),
          Expanded(
            child: Stack(
              children: [
                // The list spans the window (scrollbar on the window edge);
                // each item constrains itself to the 440px column.
                Positioned.fill(
                  child: ListView(
                    controller: _scroll,
                    // Clear the floating composer at the bottom.
                    padding: const EdgeInsets.only(top: 2, bottom: 150),
                    children: [
                      if (posts.isEmpty && ghosts.isEmpty)
                        Padding(
                          padding: const EdgeInsets.symmetric(vertical: 64),
                          child: Center(
                            child: Text(
                              'the river is quiet',
                              style: JynType.body.copyWith(
                                color: JynColors.muted,
                              ),
                            ),
                          ),
                        ),
                      for (final (index, post) in posts.indexed) ...[
                        if (index > 0)
                          const JynColumnItem(child: JynHairline(faint: true)),
                        JynColumnItem(child: PostCard(post: post)),
                      ],
                      for (final ghost in ghosts) ...[
                        const JynColumnItem(child: JynHairline(faint: true)),
                        JynColumnItem(
                          child: _GhostDoor(
                            carrier: ghost.carrierDisplayName,
                            authorProfileId: ghost.authorProfileId,
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
                // The feed dissolves under the floating pill.
                Positioned(
                  left: 0,
                  right: 0,
                  bottom: 0,
                  height: 130,
                  child: IgnorePointer(
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        gradient: LinearGradient(
                          begin: Alignment.topCenter,
                          end: Alignment.bottomCenter,
                          colors: [
                            JynColors.body.withValues(alpha: 0),
                            JynColors.body,
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
                Positioned(
                  left: 0,
                  right: 0,
                  bottom: 18,
                  child: Center(
                    child: Composer(startExpanded: widget.composerExpanded),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// A friend's heart on a stranger's post: a quiet door, not content.
class _GhostDoor extends StatelessWidget {
  const _GhostDoor({required this.carrier, required this.authorProfileId});

  final String carrier;
  final String authorProfileId;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 14),
      child: Row(
        children: [
          const Icon(
            Icons.door_front_door_outlined,
            size: 22,
            color: JynColors.muted,
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  '$carrier hearted a post by ${shortId(authorProfileId)}…',
                  style: JynType.body.copyWith(
                    fontSize: 13,
                    color: JynColors.secondary,
                  ),
                ),
                Text(
                  'not a friend yet — knock?',
                  style: JynType.meta.copyWith(color: JynColors.muted),
                ),
              ],
            ),
          ),
          const SizedBox(width: 8),
          MouseRegion(
            cursor: SystemMouseCursors.click,
            child: GestureDetector(
              onTap: () => runGuarded(
                context,
                () => requestFriendshipById(profileId: authorProfileId),
              ),
              child: Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: 12,
                  vertical: 5,
                ),
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(999),
                  border: Border.all(color: JynColors.chipOutline),
                ),
                child: Text(
                  'request',
                  style: JynType.body.copyWith(
                    fontSize: 12.5,
                    color: JynColors.mid,
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
