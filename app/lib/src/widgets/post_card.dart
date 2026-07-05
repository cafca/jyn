import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../actions.dart';
import '../providers.dart';
import '../rust/api/commands.dart';
import '../rust/state.dart';
import '../time_format.dart';
import 'media_attachment.dart';

/// One post as the river renders it: author, countdown, body, media,
/// named hearts, comments, and the owner/keeper actions.
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

  Future<void> _editDialog() async {
    final controller = TextEditingController(text: post.post.body);
    final body = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('edit post'),
        content: TextField(controller: controller, maxLines: 6, minLines: 2),
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

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final hearts = post.hearts;

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Text(post.authorDisplayName, style: theme.textTheme.titleSmall),
                const SizedBox(width: 8),
                _CountdownPill(expiresAt: post.post.expiresAt),
                if (post.post.edited) ...[
                  const SizedBox(width: 8),
                  Text(
                    'edited',
                    style: theme.textTheme.labelSmall?.copyWith(
                      color: scheme.outline,
                    ),
                  ),
                ],
                const Spacer(),
                Text(
                  visibilityLabel(post.post.visibility),
                  style: theme.textTheme.labelSmall?.copyWith(
                    color: scheme.outline,
                  ),
                ),
                if (post.isSelf) _ownPostMenu(),
              ],
            ),
            if (post.post.body.isNotEmpty) ...[
              const SizedBox(height: 6),
              SelectableText(post.post.body),
            ],
            for (final attachment in post.post.media) ...[
              const SizedBox(height: 8),
              MediaAttachmentView(
                attachment: attachment,
                postId: post.post.postId,
              ),
            ],
            const SizedBox(height: 6),
            Row(
              children: [
                if (!post.isSelf)
                  IconButton(
                    tooltip: post.heartedByMe ? 'take the heart back' : 'heart',
                    visualDensity: VisualDensity.compact,
                    onPressed: () => runGuarded(
                      context,
                      () => setHeart(
                        postAuthorProfileId: post.authorProfileId,
                        postId: post.post.postId,
                        active: !post.heartedByMe,
                      ),
                    ),
                    icon: Icon(
                      post.heartedByMe ? Icons.favorite : Icons.favorite_border,
                      size: 18,
                      color: post.heartedByMe ? scheme.primary : null,
                    ),
                  ),
                if (hearts.isNotEmpty)
                  Expanded(
                    child: Text(
                      '♥ ${hearts.map((h) => h.hearterDisplayName).join(', ')}',
                      style: theme.textTheme.labelMedium,
                      overflow: TextOverflow.ellipsis,
                    ),
                  )
                else
                  const Spacer(),
                if (!post.isSelf)
                  IconButton(
                    tooltip: post.keptByMe
                        ? 'release the keep'
                        : 'keep (a lease, not a copy)',
                    visualDensity: VisualDensity.compact,
                    onPressed: () => runGuarded(
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
                    icon: Icon(
                      post.keptByMe ? Icons.bookmark : Icons.bookmark_border,
                      size: 18,
                    ),
                  ),
                IconButton(
                  tooltip: 'comments',
                  visualDensity: VisualDensity.compact,
                  onPressed: () => setState(() => _threadOpen = !_threadOpen),
                  icon: Badge(
                    isLabelVisible: post.comments.isNotEmpty,
                    label: Text('${post.comments.length}'),
                    child: const Icon(Icons.chat_bubble_outline, size: 18),
                  ),
                ),
              ],
            ),
            if (_threadOpen) _thread(theme),
          ],
        ),
      ),
    );
  }

  Widget _ownPostMenu() {
    final ephemeral = post.post.expiresAt != null;
    return MenuAnchor(
      builder: (context, controller, _) => IconButton(
        visualDensity: VisualDensity.compact,
        onPressed: () =>
            controller.isOpen ? controller.close() : controller.open(),
        icon: const Icon(Icons.more_horiz, size: 18),
      ),
      menuChildren: [
        MenuItemButton(onPressed: _editDialog, child: const Text('edit')),
        if (ephemeral)
          MenuItemButton(
            onPressed: () => runGuarded(
              context,
              () => setPostLifetime(postId: post.post.postId),
            ),
            child: const Text('promote to settled'),
          )
        else
          SubmenuButton(
            menuChildren: [
              for (final (label, secs) in lifetimeOptions)
                if (secs != null)
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
            child: const Text('let it go…'),
          ),
        MenuItemButton(
          onPressed: _confirmDelete,
          child: const Text('delete everywhere'),
        ),
      ],
    );
  }

  Widget _thread(ThemeData theme) {
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (final comment in post.comments)
            Padding(
              padding: const EdgeInsets.only(bottom: 4),
              child: RichText(
                text: TextSpan(
                  style: theme.textTheme.bodyMedium,
                  children: [
                    TextSpan(
                      text: '${comment.commenterDisplayName}  ',
                      style: theme.textTheme.labelMedium?.copyWith(
                        fontWeight: FontWeight.bold,
                      ),
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
                  decoration: const InputDecoration(
                    hintText: 'add a comment…',
                    isDense: true,
                  ),
                  onSubmitted: (_) => _sendComment(),
                ),
              ),
              IconButton(
                onPressed: _sendComment,
                icon: const Icon(Icons.send, size: 18),
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
}

/// The countdown pill: ticks once a second, tinted by urgency.
class _CountdownPill extends ConsumerWidget {
  const _CountdownPill({required this.expiresAt});

  final int? expiresAt;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final scheme = Theme.of(context).colorScheme;
    final expiresAt = this.expiresAt;
    if (expiresAt == null) {
      return _pill(
        context,
        'settled',
        scheme.surfaceContainerHighest,
        scheme.onSurfaceVariant,
      );
    }
    ref.watch(clockProvider);
    final remaining = formatRemaining(nowUnixSecs(), expiresAt);
    final (background, foreground) = switch (remaining.tier) {
      UrgencyTier.critical => (scheme.errorContainer, scheme.onErrorContainer),
      UrgencyTier.warm => (
        scheme.tertiaryContainer,
        scheme.onTertiaryContainer,
      ),
      _ => (scheme.secondaryContainer, scheme.onSecondaryContainer),
    };
    return _pill(context, remaining.label, background, foreground);
  }

  Widget _pill(
    BuildContext context,
    String label,
    Color background,
    Color foreground,
  ) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
      decoration: BoxDecoration(
        color: background,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Text(
        label,
        style: Theme.of(
          context,
        ).textTheme.labelSmall?.copyWith(color: foreground),
      ),
    );
  }
}
