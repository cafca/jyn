import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../rust/domain.dart';
import '../rust/state.dart';
import '../theme/chrome.dart';
import '../theme/tokens.dart';
import '../time_format.dart';
import 'jyn_avatar.dart';
import 'lifetime_indicator.dart';
import 'media_attachment.dart';

/// One post as the river renders it: author row, media with its lifetime
/// ring, the action row, named hearts, and comments. Author controls live
/// in the `···` overflow menu.
class PostCard extends ConsumerStatefulWidget {
  const PostCard({super.key, required this.post});

  final RiverPost post;

  @override
  ConsumerState<PostCard> createState() => _PostCardState();
}

class _PostCardState extends ConsumerState<PostCard> {
  bool _threadOpen = false;
  final _commentDraft = TextEditingController();

  @override
  void dispose() {
    _commentDraft.dispose();
    super.dispose();
  }

  RiverPost get post => widget.post;

  /// The attachment that carries the lifetime ring, if any.
  MediaAttachment? get _visualMedia {
    for (final attachment in post.post.media) {
      if (attachment.kind == MediaKind.photo ||
          attachment.kind == MediaKind.video) {
        return attachment;
      }
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    final visual = _visualMedia;
    final rest = post.post.media.where((m) => m != visual).toList();

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 14),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          _authorRow(hasVisualMedia: visual != null),
          if (visual != null) ...[
            const SizedBox(height: 10),
            Stack(
              children: [
                MediaAttachmentView(
                  attachment: visual,
                  postId: post.post.postId,
                ),
                Positioned(
                  top: 10,
                  right: 10,
                  child: MediaLifetimeOverlay(
                    createdAt: post.post.createdAt,
                    expiresAt: post.post.expiresAt,
                  ),
                ),
              ],
            ),
          ],
          for (final attachment in rest) ...[
            const SizedBox(height: 10),
            MediaAttachmentView(
              attachment: attachment,
              postId: post.post.postId,
            ),
          ],
          // Without visual media the body is the post's substance and
          // renders above the action row; with media it reads as a caption
          // in the meta block below.
          if (visual == null && post.post.body.isNotEmpty) ...[
            const SizedBox(height: 10),
            _bodyBlock(),
          ],
          const SizedBox(height: 10),
          _actionRow(),
          ..._metaBlock(withCaption: visual != null),
          if (_threadOpen) _thread(),
        ],
      ),
    );
  }

  // ---- author row -----------------------------------------------------

  Widget _authorRow({required bool hasVisualMedia}) {
    return Row(
      children: [
        JynAvatar(
          profileId: post.authorProfileId,
          displayName: post.authorDisplayName,
          size: 36,
          isSelf: post.isSelf,
        ),
        const SizedBox(width: 10),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(post.authorDisplayName, style: JynType.name),
              const SizedBox(height: 1),
              _MetaLine(post: post),
            ],
          ),
        ),
        // Media posts carry the ring on the media itself.
        if (!hasVisualMedia) ...[
          LifetimePill(
            createdAt: post.post.createdAt,
            expiresAt: post.post.expiresAt,
          ),
          const SizedBox(width: 6),
        ],
        if (post.isSelf) _ownPostMenu(),
      ],
    );
  }

  // ---- body (non-media posts) -------------------------------------------

  Widget _bodyBlock() {
    final expiresAt = post.post.expiresAt;
    final bodyStyle = JynType.body.copyWith(fontSize: 15);
    if (expiresAt == null) {
      // Settled text rests on its own quiet card.
      return Container(
        width: double.infinity,
        padding: const EdgeInsets.all(16),
        decoration: BoxDecoration(
          color: JynColors.cardSettled,
          borderRadius: BorderRadius.circular(JynRadii.card),
          border: Border.all(color: JynColors.hairline),
        ),
        child: SelectableText(post.post.body, style: bodyStyle),
      );
    }
    final state = lifetimeState(
      now: nowUnixSecs(),
      createdAt: post.post.createdAt,
      expiresAt: expiresAt,
    );
    if (state.tier != UrgencyTier.normal) {
      // Draining: the amber card with its dashed border.
      return CustomPaint(
        painter: const _DashedBorderPainter(
          color: JynColors.drainingBorder,
          radius: JynRadii.card,
        ),
        child: Container(
          width: double.infinity,
          padding: const EdgeInsets.all(16),
          decoration: BoxDecoration(
            color: JynColors.drainingBg,
            borderRadius: BorderRadius.circular(JynRadii.card),
          ),
          child: SelectableText(
            post.post.body,
            style: bodyStyle.copyWith(color: JynColors.drainingBody),
          ),
        ),
      );
    }
    return SelectableText(post.post.body, style: bodyStyle);
  }

  // ---- action row --------------------------------------------------------

  Widget _actionRow() {
    return Row(
      children: [
        if (!post.isSelf) ...[
          _ActionIcon(
            icon: post.heartedByMe ? Icons.favorite : Icons.favorite_border,
            color: post.heartedByMe ? JynColors.heart : JynColors.text,
            tooltip: post.heartedByMe ? 'take the heart back' : 'heart',
            onTap: () => runGuarded(
              context,
              () => setHeart(
                postAuthorProfileId: post.authorProfileId,
                postId: post.post.postId,
                active: !post.heartedByMe,
              ),
            ),
          ),
          const SizedBox(width: 17),
        ],
        _ActionIcon(
          icon: Icons.chat_bubble_outline,
          tooltip: 'comments',
          onTap: () => setState(() => _threadOpen = !_threadOpen),
        ),
        const SizedBox(width: 17),
        const Upcoming(
          message: 'share by DM is coming soon',
          child: Icon(Icons.send_outlined, size: 22, color: JynColors.text),
        ),
        const Spacer(),
        if (!post.isSelf)
          _ActionIcon(
            icon: post.keptByMe ? Icons.bookmark : Icons.bookmark_border,
            tooltip: post.keptByMe
                ? 'release the keep'
                : 'keep (a lease, not a copy)',
            onTap: () => runGuarded(
              context,
              () => post.keptByMe
                  ? releaseKeep(
                      postAuthorProfileId: post.authorProfileId,
                      postId: post.post.postId,
                    )
                  : keepPost(
                      postAuthorProfileId: post.authorProfileId,
                      postId: post.post.postId,
                    ),
            ),
          ),
      ],
    );
  }

  // ---- meta block ----------------------------------------------------------

  List<Widget> _metaBlock({required bool withCaption}) {
    final hearts = post.hearts;
    final heartsLine = StringBuffer();
    if (hearts.isNotEmpty) heartsLine.write('♥ ${heartsSummary(hearts)}');
    if (post.keptByMe) {
      heartsLine.write(hearts.isEmpty ? 'kept by you' : ' · kept by you');
    }
    return [
      if (heartsLine.isNotEmpty) ...[
        const SizedBox(height: 8),
        MouseRegion(
          cursor: hearts.isNotEmpty
              ? SystemMouseCursors.click
              : MouseCursor.defer,
          child: GestureDetector(
            onTap: hearts.isNotEmpty ? _showHearts : null,
            child: Text(
              heartsLine.toString(),
              style: JynType.body.copyWith(
                fontSize: 13,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
        ),
      ],
      if (withCaption && post.post.body.isNotEmpty) ...[
        const SizedBox(height: 5),
        SelectableText.rich(
          TextSpan(
            style: JynType.body.copyWith(fontSize: 13.5),
            children: [
              TextSpan(
                text: '${post.authorDisplayName}  ',
                style: const TextStyle(fontWeight: FontWeight.w600),
              ),
              TextSpan(text: post.post.body),
            ],
          ),
        ),
      ],
      if (post.comments.isNotEmpty && !_threadOpen) ...[
        const SizedBox(height: 5),
        MouseRegion(
          cursor: SystemMouseCursors.click,
          child: GestureDetector(
            onTap: () => setState(() => _threadOpen = true),
            child: Text(
              'View ${post.comments.length} '
              'comment${post.comments.length == 1 ? '' : 's'}',
              style: JynType.body.copyWith(
                fontSize: 13,
                color: JynColors.secondary,
              ),
            ),
          ),
        ),
      ],
    ];
  }

  void _showHearts() {
    showDialog<void>(
      context: context,
      builder: (context) => AlertDialog(
        backgroundColor: JynColors.body,
        title: const Text('hearts', style: JynType.name),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final heart in post.hearts)
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 4),
                child: Row(
                  children: [
                    JynAvatar(
                      profileId: heart.hearterProfileId,
                      displayName: heart.hearterDisplayName,
                      size: 26,
                    ),
                    const SizedBox(width: 8),
                    Text(heart.hearterDisplayName, style: JynType.body),
                  ],
                ),
              ),
          ],
        ),
      ),
    );
  }

  // ---- comments -------------------------------------------------------------

  Widget _thread() {
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (final comment in post.comments)
            Padding(
              padding: const EdgeInsets.only(bottom: 4),
              child: Text.rich(
                TextSpan(
                  style: JynType.body.copyWith(fontSize: 13),
                  children: [
                    TextSpan(
                      text: '${comment.commenterDisplayName}  ',
                      style: const TextStyle(fontWeight: FontWeight.w600),
                    ),
                    TextSpan(text: comment.body),
                  ],
                ),
              ),
            ),
          Row(
            children: [
              Expanded(
                child: TextField(
                  controller: _commentDraft,
                  style: JynType.body.copyWith(fontSize: 13),
                  cursorColor: JynColors.leaf,
                  decoration: InputDecoration(
                    hintText: 'add a comment…',
                    hintStyle: JynType.body.copyWith(
                      fontSize: 13,
                      color: JynColors.muted,
                    ),
                    isDense: true,
                    border: InputBorder.none,
                  ),
                  onSubmitted: (_) => _sendComment(),
                ),
              ),
              _ActionIcon(
                icon: Icons.arrow_upward,
                size: 18,
                color: JynColors.mid,
                tooltip: 'send',
                onTap: _sendComment,
              ),
            ],
          ),
        ],
      ),
    );
  }

  Future<void> _sendComment() async {
    final body = _commentDraft.text.trim();
    if (body.isEmpty) return;
    await runGuarded(
      context,
      () => publishComment(
        postAuthorProfileId: post.authorProfileId,
        postId: post.post.postId,
        body: body,
      ),
    );
    _commentDraft.clear();
  }

  // ---- author controls (··· menu) -------------------------------------------

  Widget _ownPostMenu() {
    final ephemeral = post.post.expiresAt != null;
    return MenuAnchor(
      builder: (context, controller, _) => _ActionIcon(
        icon: Icons.more_horiz,
        size: 20,
        color: JynColors.secondary,
        tooltip: 'post options',
        onTap: () => controller.isOpen ? controller.close() : controller.open(),
      ),
      menuChildren: [
        MenuItemButton(onPressed: _editDialog, child: const Text('edit')),
        if (ephemeral)
          MenuItemButton(
            onPressed: () => runGuarded(
              context,
              () => setPostLifetime(postId: post.post.postId),
            ),
            child: const Text('make permanent'),
          )
        else
          SubmenuButton(
            menuChildren: [
              for (final (label, secs) in ephemeralLifetimeOptions)
                MenuItemButton(
                  onPressed: () => runGuarded(
                    context,
                    () => setPostLifetime(
                      postId: post.post.postId,
                      expiresAt: nowUnixSecs() + secs,
                    ),
                  ),
                  child: Text(label),
                ),
            ],
            child: const Text('make ephemeral'),
          ),
        MenuItemButton(
          onPressed: _confirmDelete,
          child: const Text(
            'delete',
            style: TextStyle(color: Color(0xFFB3402A)),
          ),
        ),
      ],
    );
  }

  Future<void> _editDialog() async {
    final controller = TextEditingController(text: post.post.body);
    final body = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        backgroundColor: JynColors.body,
        title: const Text('edit post'),
        content: TextField(
          controller: controller,
          maxLines: 6,
          minLines: 2,
          cursorColor: JynColors.leaf,
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, controller.text),
            child: const Text('save (marked as edited)'),
          ),
        ],
      ),
    );
    if (body != null && body.trim().isNotEmpty && mounted) {
      await runGuarded(
        context,
        () => editPost(postId: post.post.postId, body: body.trim()),
      );
    }
  }

  Future<void> _confirmDelete() async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        backgroundColor: JynColors.body,
        title: const Text('delete post?'),
        content: const Text(
          'The delete reaches every copy, kept ones included.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('delete'),
          ),
        ],
      ),
    );
    if (confirmed == true && mounted) {
      await runGuarded(context, () => deletePost(postId: post.post.postId));
    }
  }
}

/// The `◑ circles · 2h` line (plus `· edited`), ticking with the clock so
/// the age stays honest.
class _MetaLine extends ConsumerWidget {
  const _MetaLine({required this.post});

  final RiverPost post;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    ref.watch(clockProvider);
    final age = formatAge(nowUnixSecs(), post.post.createdAt);
    final edited = post.post.edited ? ' · edited' : '';
    return Text(
      '${visibilityLabel(post.post.visibility)} · $age$edited',
      style: JynType.meta,
    );
  }
}

/// A 22px action-row icon without Material ink.
class _ActionIcon extends StatelessWidget {
  const _ActionIcon({
    required this.icon,
    required this.onTap,
    this.color = JynColors.text,
    this.size = 22,
    this.tooltip,
  });

  final IconData icon;
  final VoidCallback onTap;
  final Color color;
  final double size;
  final String? tooltip;

  @override
  Widget build(BuildContext context) {
    final child = MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Icon(icon, size: size, color: color),
      ),
    );
    return tooltip == null ? child : Tooltip(message: tooltip!, child: child);
  }
}

/// Dashed rounded border for the draining card (Flutter has no built-in).
class _DashedBorderPainter extends CustomPainter {
  const _DashedBorderPainter({required this.color, required this.radius});

  final Color color;
  final double radius;

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.4;
    final path = Path()
      ..addRRect(
        RRect.fromRectAndRadius(Offset.zero & size, Radius.circular(radius)),
      );
    const dash = 5.0, gap = 4.0;
    for (final metric in path.computeMetrics()) {
      var distance = 0.0;
      while (distance < metric.length) {
        canvas.drawPath(metric.extractPath(distance, distance + dash), paint);
        distance += dash + gap;
      }
    }
  }

  @override
  bool shouldRepaint(_DashedBorderPainter oldDelegate) =>
      oldDelegate.color != color || oldDelegate.radius != radius;
}
