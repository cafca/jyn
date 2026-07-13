import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../rust/groups.dart';
import '../rust/groups/service.dart';
import '../theme/chrome.dart';
import '../theme/tokens.dart';
import 'group_place_screen.dart';

/// The Groups hub (ADR-0012): the dedicated destination listing the groups
/// you belong to and hosting Create-group. Friend-based suggestions join
/// with the discovery milestone.
class GroupsHubScreen extends ConsumerWidget {
  const GroupsHubScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final mine = ref.watch(myGroupsProvider);
    final suggestions = ref.watch(groupSuggestionsProvider);

    return Scaffold(
      body: Column(
        children: [
          const JynTitlebarStrip(),
          JynToolbar(
            showBack: true,
            title: 'Groups',
            actions: [
              JynToolbarIcon(
                icon: Icons.add,
                tooltip: 'create a group',
                onTap: () => _createGroup(context),
              ),
            ],
          ),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.symmetric(vertical: 22),
              children: [
                if (mine.isEmpty)
                  JynColumnItem(
                    child: Padding(
                      padding: const EdgeInsets.symmetric(vertical: 40),
                      child: Center(
                        child: Text(
                          'no groups yet — gather some people',
                          style: JynType.body.copyWith(color: JynColors.muted),
                        ),
                      ),
                    ),
                  ),
                for (final (index, group) in mine.indexed) ...[
                  if (index > 0)
                    const JynColumnItem(child: JynHairline(faint: true)),
                  JynColumnItem(child: _GroupRow(group: group)),
                ],
                // Friend-based discovery: groups your friends are in
                // (ADR-0008, ADR-0012).
                if (suggestions.isNotEmpty) ...[
                  const SizedBox(height: 26),
                  JynColumnItem(
                    child: Text(
                      'through your friends',
                      style: JynType.meta.copyWith(color: JynColors.muted),
                    ),
                  ),
                  const SizedBox(height: 6),
                  for (final suggestion in suggestions)
                    JynColumnItem(child: _SuggestionRow(suggestion: suggestion)),
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }

  Future<void> _createGroup(BuildContext context) async {
    final request = await showDialog<_CreateGroupRequest>(
      context: context,
      builder: (context) => const _CreateGroupDialog(),
    );
    if (request == null || !context.mounted) return;
    await runGuarded(
      context,
      () => createGroup(
        name: request.name,
        contentMode: request.contentMode,
        joinMode: request.joinMode,
        discoverability: request.discoverability,
      ),
    );
  }
}

class _GroupRow extends StatelessWidget {
  const _GroupRow({required this.group});

  final GroupView group;

  @override
  Widget build(BuildContext context) {
    final memberCount = group.memberCount;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => openGroupPlace(context, groupId: group.groupId),
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: 14),
          child: Row(
            children: [
              const Icon(Icons.forest_outlined, size: 22, color: JynColors.mid),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      group.name,
                      style: JynType.name,
                      overflow: TextOverflow.ellipsis,
                    ),
                    Text(
                      [
                        groupModesLabel(group),
                        if (memberCount != null)
                          memberCount == 1 ? '1 member' : '$memberCount members',
                      ].join(' · '),
                      style: JynType.meta.copyWith(color: JynColors.muted),
                    ),
                  ],
                ),
              ),
              if (group.hasNewActivity)
                Container(
                  width: 8,
                  height: 8,
                  decoration: const BoxDecoration(
                    color: JynColors.accept,
                    shape: BoxShape.circle,
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _SuggestionRow extends ConsumerWidget {
  const _SuggestionRow({required this.suggestion});

  final GroupSuggestion suggestion;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final friends = ref.watch(friendsProvider);
    String nameOf(String id) {
      for (final friend in friends) {
        if (friend.profileId == id) return friend.displayName;
      }
      return shortId(id);
    }

    final viaNames = suggestion.viaFriendProfileIds.map(nameOf).join(', ');

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 10),
      child: Row(
        children: [
          const Icon(Icons.forest_outlined, size: 22, color: JynColors.muted),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  suggestion.groupName,
                  style: JynType.name,
                  overflow: TextOverflow.ellipsis,
                ),
                Text(
                  'with $viaNames',
                  style: JynType.meta.copyWith(color: JynColors.muted),
                ),
              ],
            ),
          ),
          const SizedBox(width: 8),
          MouseRegion(
            cursor: SystemMouseCursors.click,
            child: GestureDetector(
              onTap: () {
                // Sync first so the place fills even before membership.
                runGuarded(
                  context,
                  () => syncGroup(
                    groupId: suggestion.groupId,
                    viaProfileIds: suggestion.viaFriendProfileIds,
                  ),
                );
                openGroupPlace(context, groupId: suggestion.groupId);
              },
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
                  'have a look',
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

class _CreateGroupRequest {
  const _CreateGroupRequest({
    required this.name,
    required this.contentMode,
    required this.joinMode,
    required this.discoverability,
  });

  final String name;
  final GroupContentMode contentMode;
  final GroupJoinMode joinMode;
  final GroupDiscoverability discoverability;
}

class _CreateGroupDialog extends StatefulWidget {
  const _CreateGroupDialog();

  @override
  State<_CreateGroupDialog> createState() => _CreateGroupDialogState();
}

class _CreateGroupDialogState extends State<_CreateGroupDialog> {
  final _name = TextEditingController();
  var _contentMode = GroupContentMode.public;
  var _joinMode = GroupJoinMode.open;
  var _discoverability = GroupDiscoverability.listed;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      backgroundColor: JynColors.body,
      title: const Text('create a group'),
      content: SizedBox(
        width: 360,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              controller: _name,
              autofocus: true,
              decoration: const InputDecoration(hintText: 'group name'),
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: 18),
            Text('joining', style: JynType.meta),
            const SizedBox(height: 6),
            SegmentedButton<GroupJoinMode>(
              segments: const [
                ButtonSegment(
                  value: GroupJoinMode.open,
                  label: Text('open'),
                ),
                ButtonSegment(
                  value: GroupJoinMode.request,
                  label: Text('request to join'),
                ),
              ],
              selected: {_joinMode},
              onSelectionChanged: (selection) =>
                  setState(() => _joinMode = selection.single),
            ),
            const SizedBox(height: 14),
            Text('discovery', style: JynType.meta),
            const SizedBox(height: 6),
            SegmentedButton<GroupDiscoverability>(
              segments: const [
                ButtonSegment(
                  value: GroupDiscoverability.listed,
                  label: Text('listed'),
                ),
                ButtonSegment(
                  value: GroupDiscoverability.unlisted,
                  label: Text('unlisted'),
                ),
              ],
              selected: {_discoverability},
              onSelectionChanged: (selection) =>
                  setState(() => _discoverability = selection.single),
            ),
            const SizedBox(height: 14),
            Text('who can read', style: JynType.meta),
            const SizedBox(height: 6),
            SegmentedButton<GroupContentMode>(
              segments: const [
                ButtonSegment(
                  value: GroupContentMode.public,
                  label: Text('public'),
                ),
                ButtonSegment(
                  value: GroupContentMode.membersOnly,
                  label: Text('members-only'),
                ),
              ],
              selected: {_contentMode},
              onSelectionChanged: (selection) =>
                  setState(() => _contentMode = selection.single),
            ),
            const SizedBox(height: 6),
            // Content mode is fixed forever at creation (ADR-0006).
            Text(
              'fixed once created',
              style: JynType.meta.copyWith(color: JynColors.muted),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('cancel'),
        ),
        FilledButton(
          onPressed: _name.text.trim().isEmpty
              ? null
              : () => Navigator.pop(
                  context,
                  _CreateGroupRequest(
                    name: _name.text.trim(),
                    contentMode: _contentMode,
                    joinMode: _joinMode,
                    discoverability: _discoverability,
                  ),
                ),
          child: const Text('create'),
        ),
      ],
    );
  }
}

/// "open · listed" — the compact mode line used on hub rows and headers.
String groupModesLabel(GroupView group) {
  final joining = switch (group.joinMode) {
    GroupJoinMode.open => 'open',
    GroupJoinMode.request => 'request to join',
  };
  final content = switch (group.contentMode) {
    GroupContentMode.public => 'public',
    GroupContentMode.membersOnly => 'members-only',
  };
  return '$content · $joining';
}
