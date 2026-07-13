import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../rust/groups.dart';
import '../rust/groups/service.dart';
import '../theme/chrome.dart';
import '../theme/tokens.dart';
import '../widgets/jyn_avatar.dart';

/// The Group admin sub-view (ADR-0013): the Owner's dedicated governance
/// surface — metadata, pending requests, members, and ownership transfer.
class GroupAdminScreen extends ConsumerWidget {
  const GroupAdminScreen({super.key, required this.groupId});

  final String groupId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final group = ref.watch(groupProvider(groupId));
    final isOwner = group?.viewerStatus == GroupViewerStatus.owner;

    return Scaffold(
      body: Column(
        children: [
          const JynTitlebarStrip(),
          JynToolbar(showBack: true, title: group?.name ?? 'Group admin'),
          Expanded(
            child: group == null || !isOwner
                ? Center(
                    child: Text(
                      'the owner governs here',
                      style: JynType.body.copyWith(color: JynColors.muted),
                    ),
                  )
                : ListView(
                    padding: const EdgeInsets.symmetric(vertical: 22),
                    children: [
                      JynColumnItem(child: _MetadataSection(group: group)),
                      const SizedBox(height: 22),
                      const JynColumnItem(child: JynHairline(faint: true)),
                      if (group.pendingRequests.isNotEmpty) ...[
                        JynColumnItem(child: _RequestsSection(group: group)),
                        const JynColumnItem(child: JynHairline(faint: true)),
                      ],
                      JynColumnItem(child: _MembersSection(group: group)),
                    ],
                  ),
          ),
        ],
      ),
    );
  }
}

class _MetadataSection extends ConsumerStatefulWidget {
  const _MetadataSection({required this.group});

  final GroupView group;

  @override
  ConsumerState<_MetadataSection> createState() => _MetadataSectionState();
}

class _MetadataSectionState extends ConsumerState<_MetadataSection> {
  late final _name = TextEditingController(text: widget.group.name);

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final group = widget.group;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text('identity', style: JynType.meta),
        const SizedBox(height: 8),
        Row(
          children: [
            Expanded(
              child: TextField(
                controller: _name,
                style: JynType.body,
                decoration: const InputDecoration(hintText: 'group name'),
                onSubmitted: (_) => _saveName(),
              ),
            ),
            const SizedBox(width: 10),
            TextButton(onPressed: _saveName, child: const Text('rename')),
          ],
        ),
        const SizedBox(height: 18),
        Text('joining', style: JynType.meta),
        const SizedBox(height: 6),
        SegmentedButton<GroupJoinMode>(
          segments: const [
            ButtonSegment(value: GroupJoinMode.open, label: Text('open')),
            ButtonSegment(
              value: GroupJoinMode.request,
              label: Text('request to join'),
            ),
          ],
          selected: {group.joinMode},
          onSelectionChanged: (selection) => runGuarded(
            context,
            () => editGroupMetadata(
              groupId: group.groupId,
              joinMode: selection.single,
            ),
          ),
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
          selected: {group.discoverability},
          onSelectionChanged: (selection) => runGuarded(
            context,
            () => editGroupMetadata(
              groupId: group.groupId,
              discoverability: selection.single,
            ),
          ),
        ),
        const SizedBox(height: 10),
        Text(
          // Content mode is fixed forever at creation (ADR-0006).
          switch (group.contentMode) {
            GroupContentMode.public => 'public — fixed at creation',
            GroupContentMode.membersOnly => 'members-only — fixed at creation',
          },
          style: JynType.meta.copyWith(color: JynColors.muted),
        ),
      ],
    );
  }

  void _saveName() {
    final name = _name.text.trim();
    if (name.isEmpty || name == widget.group.name) return;
    runGuarded(
      context,
      () => editGroupMetadata(groupId: widget.group.groupId, name: name),
    );
  }
}

class _RequestsSection extends StatelessWidget {
  const _RequestsSection({required this.group});

  final GroupView group;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 18),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('asking to join', style: JynType.meta),
          const SizedBox(height: 8),
          for (final request in group.pendingRequests)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 6),
              child: Row(
                children: [
                  JynAvatar(
                    profileId: request.requesterProfileId,
                    displayName: request.requesterDisplayName,
                    size: 26,
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          request.requesterDisplayName,
                          style: JynType.name,
                        ),
                        if (request.greeting != null)
                          Text(
                            request.greeting!,
                            style: JynType.meta,
                            overflow: TextOverflow.ellipsis,
                          ),
                      ],
                    ),
                  ),
                  TextButton(
                    onPressed: () => runGuarded(
                      context,
                      () => respondGroupJoin(
                        groupId: group.groupId,
                        requesterProfileId: request.requesterProfileId,
                        accept: true,
                      ),
                    ),
                    child: const Text('let in'),
                  ),
                  TextButton(
                    onPressed: () => runGuarded(
                      context,
                      () => respondGroupJoin(
                        groupId: group.groupId,
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
      ),
    );
  }
}

class _MembersSection extends ConsumerWidget {
  const _MembersSection({required this.group});

  final GroupView group;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final myId = ref.watch(profileProvider)?.profileId;
    final friends = ref.watch(friendsProvider);
    String nameOf(String profileId) {
      if (profileId == myId) return 'you';
      for (final friend in friends) {
        if (friend.profileId == profileId) return friend.displayName;
      }
      return shortId(profileId);
    }

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 18),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('members', style: JynType.meta),
          const SizedBox(height: 8),
          for (final member in group.members)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 6),
              child: Row(
                children: [
                  JynAvatar(
                    profileId: member.profileId,
                    displayName: nameOf(member.profileId),
                    size: 26,
                    isSelf: member.profileId == myId,
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      nameOf(member.profileId),
                      style: JynType.name,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  if (member.roles.contains(GroupRole.owner))
                    Text('owner', style: JynType.meta)
                  else ...[
                    TextButton(
                      onPressed: () => _confirmTransfer(
                        context,
                        member.profileId,
                        nameOf(member.profileId),
                      ),
                      child: const Text('hand over'),
                    ),
                    TextButton(
                      onPressed: () => _confirmRemove(
                        context,
                        member.profileId,
                        nameOf(member.profileId),
                      ),
                      child: const Text('remove'),
                    ),
                  ],
                ],
              ),
            ),
        ],
      ),
    );
  }

  Future<void> _confirmRemove(
    BuildContext context,
    String profileId,
    String name,
  ) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        backgroundColor: JynColors.body,
        title: Text('remove $name?'),
        content: const Text('They keep only what already reached them.'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('remove'),
          ),
        ],
      ),
    );
    if (confirmed == true && context.mounted) {
      await runGuarded(
        context,
        () => removeGroupMember(
          groupId: group.groupId,
          memberProfileId: profileId,
        ),
      );
    }
  }

  Future<void> _confirmTransfer(
    BuildContext context,
    String profileId,
    String name,
  ) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        backgroundColor: JynColors.body,
        title: Text('hand the group to $name?'),
        content: const Text(
          'They become the owner; you stay a member until you leave.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('hand over'),
          ),
        ],
      ),
    );
    if (confirmed == true && context.mounted) {
      await runGuarded(
        context,
        () => transferGroupOwnership(
          groupId: group.groupId,
          toProfileId: profileId,
        ),
      );
    }
  }
}
