import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../rust/runtime.dart';
import '../rust/state.dart';
import '../theme/chrome.dart';
import '../theme/tokens.dart';
import '../widgets/composer.dart';
import '../widgets/jyn_avatar.dart';
import '../widgets/post_card.dart';
import 'group_place_screen.dart';
import 'groups_hub_screen.dart';

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
    final doors = ref.watch(groupDoorsProvider);
    final groupCards = ref.watch(groupCardsProvider);
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
              JynToolbarIcon(
                icon: Icons.forest_outlined,
                tooltip: 'groups',
                onTap: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => const GroupsHubScreen(),
                  ),
                ),
              ),
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
                      if (posts.isEmpty &&
                          ghosts.isEmpty &&
                          doors.isEmpty &&
                          groupCards.isEmpty)
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
                      // Digest doors sort into the reverse-chron river by
                      // the group's latest activity (ADR-0010).
                      for (final (index, entry) in _riverEntries(
                        posts,
                        doors,
                      ).indexed) ...[
                        if (index > 0)
                          const JynColumnItem(child: JynHairline(faint: true)),
                        JynColumnItem(
                          child: switch (entry) {
                            (final RiverPost post, _) => PostCard(post: post),
                            (_, final GroupDigestDoor door) => _GroupDoor(
                              door: door,
                            ),
                            _ => const SizedBox.shrink(),
                          },
                        ),
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
                      // A friend's heart on a public+listed group post: a
                      // named door into the group (ADR-0009).
                      for (final card in groupCards) ...[
                        const JynColumnItem(child: JynHairline(faint: true)),
                        JynColumnItem(child: _GroupHeartCard(card: card)),
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

/// Posts and group digest doors merged reverse-chronologically: exactly one
/// of each pair's fields is set.
List<(RiverPost?, GroupDigestDoor?)> _riverEntries(
  List<RiverPost> posts,
  List<GroupDigestDoor> doors,
) {
  final entries = <(RiverPost?, GroupDigestDoor?)>[
    for (final post in posts) (post, null),
    for (final door in doors) (null, door),
  ];
  entries.sort((a, b) {
    final aTime = a.$1?.post.createdAt ?? a.$2!.latestActivityAt;
    final bTime = b.$1?.post.createdAt ?? b.$2!.latestActivityAt;
    return bTime.compareTo(aTime);
  });
  return entries;
}

/// One river entry per member-group with new activity, opening the group
/// place — group posts never interleave individually (ADR-0010).
class _GroupDoor extends StatelessWidget {
  const _GroupDoor({required this.door});

  final GroupDigestDoor door;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => openGroupPlace(context, groupId: door.groupId),
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: 14),
          child: Row(
            children: [
              const Icon(
                Icons.forest_outlined,
                size: 22,
                color: JynColors.muted,
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      'new in ${door.name}',
                      style: JynType.body.copyWith(
                        fontSize: 13,
                        color: JynColors.secondary,
                      ),
                    ),
                    Text(
                      'step in?',
                      style: JynType.meta.copyWith(color: JynColors.muted),
                    ),
                  ],
                ),
              ),
              const Icon(Icons.chevron_right, size: 18, color: JynColors.muted),
            ],
          ),
        ),
      ),
    );
  }
}

/// "♥ Bob, in *Group X*" — a friend's heart on a public+listed group post,
/// pointing into the group place; the post is not copied (ADR-0009).
class _GroupHeartCard extends StatelessWidget {
  const _GroupHeartCard({required this.card});

  final GroupDiscoveryCard card;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () {
          // The carrier seeds reach into the group topic before the visit.
          runGuarded(
            context,
            () => syncGroup(
              groupId: card.groupId,
              viaProfileIds: [card.carrierProfileId],
            ),
          );
          openGroupPlace(context, groupId: card.groupId);
        },
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: 14),
          child: Row(
            children: [
              const Icon(
                Icons.favorite_border,
                size: 20,
                color: JynColors.muted,
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      '${card.carrierDisplayName} hearted a post in '
                      '${card.groupName}',
                      style: JynType.body.copyWith(
                        fontSize: 13,
                        color: JynColors.secondary,
                      ),
                    ),
                    Text(
                      'step in?',
                      style: JynType.meta.copyWith(color: JynColors.muted),
                    ),
                  ],
                ),
              ),
              const Icon(Icons.chevron_right, size: 18, color: JynColors.muted),
            ],
          ),
        ),
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
