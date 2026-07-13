import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../media_limits.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../rust/domain.dart';
import '../rust/groups.dart';
import '../rust/groups/service.dart';
import '../theme/chrome.dart';
import '../theme/tokens.dart';
import '../time_format.dart';
import '../widgets/jyn_avatar.dart';
import '../widgets/media_attachment.dart';
import 'group_admin_screen.dart';
import 'groups_hub_screen.dart';

/// Opens a group's place screen (from the hub, a river digest door, or a
/// discovery card), marking the visit so the door clears.
void openGroupPlace(BuildContext context, {required String groupId}) {
  final routeName = 'jyn-group:$groupId';
  if (ModalRoute.of(context)?.settings.name == routeName) return;
  runGuarded(context, () => markGroupOpened(groupId: groupId));
  Navigator.of(context).push(
    MaterialPageRoute<void>(
      settings: RouteSettings(name: routeName),
      builder: (_) => GroupPlaceScreen(groupId: groupId),
    ),
  );
}

/// The Group place (ADR-0013): one group's identity, stream, and — for
/// members — the in-group composer. Adapts to the viewer's status; Owner
/// governance lives in the dedicated admin sub-view.
class GroupPlaceScreen extends ConsumerWidget {
  const GroupPlaceScreen({super.key, required this.groupId});

  final String groupId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final group = ref.watch(groupProvider(groupId));

    return Scaffold(
      body: Column(
        children: [
          const JynTitlebarStrip(),
          JynToolbar(
            showBack: true,
            title: group?.name ?? 'Group',
            actions: [
              if (group?.viewerStatus == GroupViewerStatus.owner)
                JynToolbarIcon(
                  icon: Icons.tune,
                  tooltip: 'group admin',
                  onTap: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => GroupAdminScreen(groupId: groupId),
                    ),
                  ),
                ),
            ],
          ),
          Expanded(
            child: group == null
                ? Center(
                    child: Text(
                      'reaching for this group…',
                      style: JynType.body.copyWith(color: JynColors.muted),
                    ),
                  )
                : _GroupPlaceBody(group: group),
          ),
        ],
      ),
    );
  }
}

class _GroupPlaceBody extends ConsumerWidget {
  const _GroupPlaceBody({required this.group});

  final GroupView group;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final isMember =
        group.viewerStatus == GroupViewerStatus.owner ||
        group.viewerStatus == GroupViewerStatus.member;
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final posts = group.posts.where((post) => !isExpired(post, now)).toList();

    return Stack(
      children: [
        Positioned.fill(
          child: ListView(
            padding: EdgeInsets.only(top: 22, bottom: isMember ? 150 : 22),
            children: [
              JynColumnItem(child: _GroupHeader(group: group)),
              const SizedBox(height: 22),
              const JynColumnItem(child: JynHairline(faint: true)),
              if (posts.isEmpty)
                JynColumnItem(
                  child: Padding(
                    padding: const EdgeInsets.symmetric(vertical: 40),
                    child: Center(
                      child: Text(
                        group.contentMode == GroupContentMode.membersOnly &&
                                !isMember
                            ? 'the stream is for members'
                            : 'nothing here yet',
                        style: JynType.body.copyWith(color: JynColors.muted),
                      ),
                    ),
                  ),
                ),
              for (final (index, post) in posts.indexed) ...[
                if (index > 0)
                  const JynColumnItem(child: JynHairline(faint: true)),
                JynColumnItem(
                  child: _GroupPostCard(group: group, post: post),
                ),
              ],
            ],
          ),
        ),
        if (isMember)
          Positioned(
            left: 0,
            right: 0,
            bottom: 18,
            child: Center(child: _GroupComposer(groupId: group.groupId)),
          ),
      ],
    );
  }
}

bool isExpired(ReducedPost post, int now) {
  final expiresAt = post.expiresAt;
  return expiresAt != null && expiresAt <= now;
}

class _GroupHeader extends ConsumerWidget {
  const _GroupHeader({required this.group});

  final GroupView group;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final memberCount = group.memberCount;
    final statusLine = switch (group.viewerStatus) {
      GroupViewerStatus.owner => 'you hold this place',
      GroupViewerStatus.member => 'you are a member',
      GroupViewerStatus.pending => 'asked to join — awaiting the owner',
      GroupViewerStatus.nonMember => 'not a member',
    };

    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const Icon(Icons.forest_outlined, size: 44, color: JynColors.mid),
        const SizedBox(width: 16),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                group.name,
                style: const TextStyle(
                  fontSize: 20,
                  fontWeight: FontWeight.w700,
                  color: JynColors.ink,
                ),
              ),
              const SizedBox(height: 4),
              Text(
                [
                  groupModesLabel(group),
                  if (memberCount != null)
                    memberCount == 1 ? '1 member' : '$memberCount members',
                ].join(' · '),
                style: JynType.meta,
              ),
              Text(statusLine, style: JynType.meta),
            ],
          ),
        ),
        const SizedBox(width: 8),
        switch (group.viewerStatus) {
          GroupViewerStatus.nonMember => _HeaderAction(
            label: group.joinMode == GroupJoinMode.open ? 'join' : 'request',
            onTap: () => runGuarded(
              context,
              () => joinGroup(groupId: group.groupId, viaProfileIds: []),
            ),
          ),
          GroupViewerStatus.member => _HeaderAction(
            label: 'leave',
            onTap: () => _confirmLeave(context),
          ),
          _ => const SizedBox.shrink(),
        },
      ],
    );
  }

  Future<void> _confirmLeave(BuildContext context) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        backgroundColor: JynColors.body,
        title: Text('leave ${group.name}?'),
        content: const Text('The record of your time here remains.'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('leave'),
          ),
        ],
      ),
    );
    if (confirmed == true && context.mounted) {
      await runGuarded(context, () => leaveGroup(groupId: group.groupId));
    }
  }
}

class _HeaderAction extends StatelessWidget {
  const _HeaderAction({required this.label, required this.onTap});

  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        onTap: onTap,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(999),
            border: Border.all(color: JynColors.chipOutline),
          ),
          child: Text(
            label,
            style: JynType.body.copyWith(fontSize: 12.5, color: JynColors.mid),
          ),
        ),
      ),
    );
  }
}

/// A group post: same single post type as everywhere, rendered lean. The
/// group's Content mode is the fixed visibility, so no reach glyph.
class _GroupPostCard extends ConsumerStatefulWidget {
  const _GroupPostCard({required this.group, required this.post});

  final GroupView group;
  final ReducedPost post;

  @override
  ConsumerState<_GroupPostCard> createState() => _GroupPostCardState();
}

class _GroupPostCardState extends ConsumerState<_GroupPostCard> {
  final _comment = TextEditingController();

  @override
  void dispose() {
    _comment.dispose();
    super.dispose();
  }

  String _nameOf(String profileId, WidgetRef ref) {
    final profile = ref.read(profileProvider);
    if (profile?.profileId == profileId) return profile!.displayName;
    for (final friend in ref.read(friendsProvider)) {
      if (friend.profileId == profileId) return friend.displayName;
    }
    return shortId(profileId);
  }

  @override
  Widget build(BuildContext context) {
    final group = widget.group;
    final post = widget.post;
    final myId = ref.watch(profileProvider)?.profileId;
    final isSelf = post.profileId == myId;
    final isMember =
        group.viewerStatus == GroupViewerStatus.owner ||
        group.viewerStatus == GroupViewerStatus.member;
    final comments = group.comments
        .where((comment) => comment.postId == post.postId)
        .toList();
    final hearts = group.hearts
        .where((heart) => heart.postId == post.postId)
        .toList();
    final heartedByMe = hearts.any((heart) => heart.hearterProfileId == myId);
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final authorName = _nameOf(post.profileId, ref);

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 14),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              JynAvatar(
                profileId: post.profileId,
                displayName: authorName,
                size: 26,
                isSelf: isSelf,
              ),
              const SizedBox(width: 10),
              Text(authorName, style: JynType.name),
              const SizedBox(width: 8),
              Text(formatAge(now, post.createdAt), style: JynType.meta),
              if (post.edited) ...[
                const SizedBox(width: 6),
                Text('edited', style: JynType.meta),
              ],
              const Spacer(),
              if (isSelf)
                JynToolbarIcon(
                  icon: Icons.more_horiz,
                  tooltip: 'your post',
                  onTap: () => _ownPostMenu(context),
                ),
            ],
          ),
          const SizedBox(height: 8),
          Text(post.body, style: JynType.body),
          for (final attachment in post.media) ...[
            const SizedBox(height: 10),
            MediaAttachmentView(attachment: attachment, postId: post.postId),
          ],
          const SizedBox(height: 8),
          Row(
            children: [
              if (isMember)
                MouseRegion(
                  cursor: SystemMouseCursors.click,
                  child: GestureDetector(
                    onTap: () => runGuarded(
                      context,
                      () => setGroupHeart(
                        groupId: group.groupId,
                        postAuthorProfileId: post.profileId,
                        postId: post.postId,
                        active: !heartedByMe,
                      ),
                    ),
                    child: Icon(
                      heartedByMe ? Icons.favorite : Icons.favorite_border,
                      size: 16,
                      color: heartedByMe ? JynColors.heart : JynColors.muted,
                    ),
                  ),
                ),
              if (hearts.isNotEmpty) ...[
                const SizedBox(width: 6),
                Text(
                  hearts
                      .map((heart) => _nameOf(heart.hearterProfileId, ref))
                      .join(', '),
                  style: JynType.meta,
                ),
              ],
            ],
          ),
          for (final comment in comments)
            Padding(
              padding: const EdgeInsets.only(top: 8, left: 12),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    _nameOf(comment.commenterProfileId, ref),
                    style: JynType.meta.copyWith(color: JynColors.secondary),
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      comment.body,
                      style: JynType.body.copyWith(fontSize: 13),
                    ),
                  ),
                ],
              ),
            ),
          if (isMember)
            Padding(
              padding: const EdgeInsets.only(top: 8, left: 12),
              child: TextField(
                controller: _comment,
                style: JynType.body.copyWith(fontSize: 13),
                decoration: const InputDecoration(
                  hintText: 'answer…',
                  isDense: true,
                  border: InputBorder.none,
                ),
                onSubmitted: (value) {
                  final body = value.trim();
                  if (body.isEmpty) return;
                  _comment.clear();
                  runGuarded(
                    context,
                    () => publishGroupComment(
                      groupId: group.groupId,
                      postAuthorProfileId: post.profileId,
                      postId: post.postId,
                      body: body,
                    ),
                  );
                },
              ),
            ),
        ],
      ),
    );
  }

  Future<void> _ownPostMenu(BuildContext context) async {
    final action = await showModalBottomSheet<String>(
      context: context,
      backgroundColor: JynColors.body,
      builder: (context) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              leading: const Icon(Icons.delete_outline),
              title: const Text('let it go'),
              onTap: () => Navigator.pop(context, 'delete'),
            ),
          ],
        ),
      ),
    );
    if (action == 'delete' && context.mounted) {
      await runGuarded(
        context,
        () => deleteGroupPost(
          groupId: widget.group.groupId,
          postId: widget.post.postId,
        ),
      );
    }
  }
}

/// The in-group composer: words and a lifetime — no visibility dial, the
/// Group's Content mode is the fixed visibility (ADR-0013).
class _GroupComposer extends ConsumerStatefulWidget {
  const _GroupComposer({required this.groupId});

  final String groupId;

  @override
  ConsumerState<_GroupComposer> createState() => _GroupComposerState();
}

class _GroupComposerState extends ConsumerState<_GroupComposer> {
  final _body = TextEditingController();
  final List<MediaDraftInput> _attachments = [];
  int? _lifetimeSecs = 24 * 3600;
  var _casting = false;

  @override
  void dispose() {
    _body.dispose();
    super.dispose();
  }

  Future<void> _attachFiles() async {
    final files = await openFiles();
    if (files.isEmpty) return;
    // Oversized media never enters the blob store (see media_limits.dart).
    final rejected = <String>[];
    for (final f in files) {
      if (exceedsLimit(kindForPath(f.path), await f.length())) {
        rejected.add(f.name);
      } else {
        _attachments.add(MediaDraftInput(path: f.path));
      }
    }
    if (rejected.isNotEmpty && mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('${rejected.join(', ')} too large')),
      );
    }
    if (mounted) setState(() {});
  }

  Future<void> _cast() async {
    final body = _body.text.trim();
    if ((body.isEmpty && _attachments.isEmpty) || _casting) return;
    setState(() => _casting = true);
    // The fields clear only on success: statements after the awaited publish
    // are skipped when it throws (runGuarded surfaces the error to a snackbar).
    await runGuarded(context, () async {
      await publishGroupPost(
        groupId: widget.groupId,
        body: body,
        lifetimeSecs: _lifetimeSecs,
        media: List.of(_attachments),
      );
      _body.clear();
      _attachments.clear();
    });
    if (mounted) setState(() => _casting = false);
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      width: 440,
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
      decoration: BoxDecoration(
        color: JynColors.body,
        borderRadius: BorderRadius.circular(JynRadii.pill),
        border: Border.all(color: JynColors.hairline),
        boxShadow: const [
          BoxShadow(
            color: Color(0x14000000),
            blurRadius: 18,
            offset: Offset(0, 6),
          ),
        ],
      ),
      child: Row(
        children: [
          Expanded(
            child: TextField(
              controller: _body,
              style: JynType.body,
              decoration: const InputDecoration(
                hintText: 'cast into this group…',
                isDense: true,
                border: InputBorder.none,
              ),
              onSubmitted: (_) => _cast(),
            ),
          ),
          const SizedBox(width: 8),
          JynToolbarIcon(
            icon: _attachments.isEmpty ? Icons.attach_file : Icons.attachment,
            tooltip: _attachments.isEmpty
                ? 'attach'
                : '${_attachments.length} attached',
            onTap: _attachFiles,
          ),
          const SizedBox(width: 8),
          // Lifetime stays a per-post choice; visibility does not exist here.
          MenuAnchor(
            builder: (context, controller, _) => MouseRegion(
              cursor: SystemMouseCursors.click,
              child: GestureDetector(
                onTap: () =>
                    controller.isOpen ? controller.close() : controller.open(),
                child: Text(
                  lifetimeOptions
                      .firstWhere((option) => option.$2 == _lifetimeSecs)
                      .$1,
                  style: JynType.meta.copyWith(color: JynColors.mid),
                ),
              ),
            ),
            menuChildren: [
              for (final (label, secs) in lifetimeOptions)
                MenuItemButton(
                  onPressed: () => setState(() => _lifetimeSecs = secs),
                  child: Text(label, style: JynType.body),
                ),
            ],
          ),
          const SizedBox(width: 10),
          JynToolbarIcon(
            icon: Icons.arrow_upward,
            tooltip: 'cast',
            onTap: _cast,
          ),
        ],
      ),
    );
  }
}
