import 'dart:io';

import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:url_launcher/url_launcher.dart';

import '../media_limits.dart';
import '../providers.dart';
import '../rust/api/media.dart' as rust_media;
import '../rust/domain.dart';
import '../theme/tokens.dart';
import 'video_attachment.dart';
import 'voice_note_player.dart';

/// Renders one attachment by kind. Blobs fetch on demand: the widget asks
/// the core once, shows a quiet placeholder, and rebuilds when MediaReady
/// lands. Photo and video render as the design's fixed 4:5 cover tile
/// (radius 14); audio and files keep their own card layouts.
class MediaAttachmentView extends ConsumerWidget {
  const MediaAttachmentView({
    super.key,
    required this.attachment,
    required this.postId,
  });

  final MediaAttachment attachment;
  final String postId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Oversized media is never fetched: a single huge file would otherwise
    // dominate (and thrash) the media cache. Show a static "too large" tile
    // instead. Enforced at post time too, but this also covers media posted
    // by other peers (see media_limits.dart).
    if (exceedsLimit(attachment.kind, attachment.byteLen)) {
      return _mediaTile(_TooLargePlaceholder(kind: attachment.kind));
    }

    final path = ref.watch(
      mediaPathsProvider.select((paths) => paths[attachment.blobHash]),
    );
    if (path == null) {
      // Idempotent: the core dedupes in-flight fetches.
      rust_media.requestMedia(blobHash: attachment.blobHash);
    }

    return switch (attachment.kind) {
      MediaKind.photo => _mediaTile(_photo(path)),
      MediaKind.audio => VoiceNotePlayer(
        waveform: attachment.waveform,
        durationMs: attachment.durationMs,
        path: path,
      ),
      MediaKind.video => _mediaTile(
        VideoAttachment(
          attachment: attachment,
          path: path,
          playerKey: '$postId:${attachment.blobHash}',
        ),
      ),
      MediaKind.file => _FileCard(attachment: attachment, path: path),
    };
  }

  /// The design's media frame: 4:5, cover-cropped, radius 14.
  Widget _mediaTile(Widget child) {
    return ClipRRect(
      borderRadius: BorderRadius.circular(JynRadii.media),
      child: AspectRatio(aspectRatio: 4 / 5, child: child),
    );
  }

  Widget _photo(String? path) {
    if (path == null) return const MediaLoadingPlaceholder();
    return Image.file(
      File(path),
      fit: BoxFit.cover,
      errorBuilder: (context, _, _) =>
          const MediaLoadingPlaceholder(icon: Icons.broken_image_outlined),
    );
  }
}

/// A quiet near-white loading state (deliberately not the mock's striped
/// placeholder) that fills whatever frame it sits in.
class MediaLoadingPlaceholder extends StatelessWidget {
  const MediaLoadingPlaceholder({super.key, this.icon = Icons.image_outlined});

  final IconData icon;

  @override
  Widget build(BuildContext context) {
    return Container(
      color: JynColors.cardGrey,
      child: Center(child: Icon(icon, size: 28, color: JynColors.muted)),
    );
  }
}

/// Shown in place of media that exceeds its size limit: it is never fetched,
/// so there is no file to render. Fills the 4:5 media frame.
class _TooLargePlaceholder extends StatelessWidget {
  const _TooLargePlaceholder({required this.kind});

  final MediaKind kind;

  @override
  Widget build(BuildContext context) {
    final limit = limitLabelForKind(kind);
    return Container(
      color: JynColors.cardGrey,
      padding: const EdgeInsets.all(12),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(
            kind == MediaKind.video
                ? Icons.videocam_off_outlined
                : Icons.hide_image_outlined,
            size: 28,
            color: JynColors.muted,
          ),
          const SizedBox(height: 8),
          Text(
            limit == null
                ? 'Too large to show'
                : 'Too large to show (over $limit)',
            textAlign: TextAlign.center,
            style: JynType.metaMono.copyWith(color: JynColors.muted),
          ),
        ],
      ),
    );
  }
}

/// A file attachment: paperclip, name, size; opens externally once local.
class _FileCard extends StatelessWidget {
  const _FileCard({required this.attachment, required this.path});

  final MediaAttachment attachment;
  final String? path;

  @override
  Widget build(BuildContext context) {
    final name = attachment.fileName ?? attachment.blobHash.substring(0, 8);
    final fetching = path == null;
    return MouseRegion(
      cursor: fetching ? SystemMouseCursors.basic : SystemMouseCursors.click,
      child: GestureDetector(
        onTap: fetching ? null : () => launchUrl(Uri.file(path!)),
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
          decoration: BoxDecoration(
            color: JynColors.field,
            borderRadius: BorderRadius.circular(JynRadii.attach),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                fetching ? Icons.downloading : Icons.attach_file,
                size: 16,
                color: JynColors.slate,
              ),
              const SizedBox(width: 8),
              Flexible(
                child: Text(
                  name,
                  overflow: TextOverflow.ellipsis,
                  style: JynType.body.copyWith(fontSize: 13),
                ),
              ),
              const SizedBox(width: 8),
              Text(_formatBytes(attachment.byteLen), style: JynType.metaMono),
            ],
          ),
        ),
      ),
    );
  }
}

String _formatBytes(int bytes) {
  if (bytes >= 1024 * 1024) {
    return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
  }
  if (bytes >= 1024) return '${(bytes / 1024).toStringAsFixed(0)} kB';
  return '$bytes B';
}
