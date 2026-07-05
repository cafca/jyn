import 'dart:io';

import 'package:flutter/material.dart' hide Visibility;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:url_launcher/url_launcher.dart';

import '../providers.dart';
import '../rust/api/media.dart' as rust_media;
import '../rust/domain.dart';
import 'video_attachment.dart';
import 'voice_note_player.dart';

/// Renders one attachment by kind. Blobs fetch on demand: the widget asks
/// the core once, shows a placeholder, and rebuilds when MediaReady lands.
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
    final path = ref.watch(
      mediaPathsProvider.select((paths) => paths[attachment.blobHash]),
    );
    if (path == null) {
      // Idempotent: the core dedupes in-flight fetches.
      rust_media.requestMedia(blobHash: attachment.blobHash);
    }

    return switch (attachment.kind) {
      MediaKind.photo => _photo(context, path),
      MediaKind.audio => VoiceNotePlayer(attachment: attachment, path: path),
      MediaKind.video => VideoAttachment(
          attachment: attachment,
          path: path,
          playerKey: '$postId:${attachment.blobHash}',
        ),
      MediaKind.file => _fileChip(context, path),
    };
  }

  Widget _photo(BuildContext context, String? path) {
    if (path == null) {
      return _placeholderBox(
        context,
        width: attachment.width?.toDouble(),
        height: attachment.height?.toDouble(),
        child: const Icon(Icons.image_outlined),
      );
    }
    return ClipRRect(
      borderRadius: BorderRadius.circular(8),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxHeight: 360),
        child: Image.file(
          File(path),
          fit: BoxFit.contain,
          alignment: Alignment.centerLeft,
          errorBuilder: (context, _, _) => _placeholderBox(
            context,
            child: const Icon(Icons.broken_image_outlined),
          ),
        ),
      ),
    );
  }

  Widget _fileChip(BuildContext context, String? path) {
    final name = attachment.fileName ?? attachment.blobHash.substring(0, 8);
    final size = _formatBytes(attachment.byteLen);
    return ActionChip(
      avatar: Icon(
        path == null ? Icons.downloading : Icons.insert_drive_file_outlined,
        size: 18,
      ),
      label: Text('$name · $size'),
      onPressed: path == null
          ? null
          : () async {
              // A named copy so the OS routes it to the right app.
              final uri = Uri.file(path);
              await launchUrl(uri);
            },
    );
  }

  Widget _placeholderBox(
    BuildContext context, {
    double? width,
    double? height,
    required Widget child,
  }) {
    final aspect = (width != null && height != null && height > 0)
        ? width / height
        : 16 / 9;
    return ClipRRect(
      borderRadius: BorderRadius.circular(8),
      child: AspectRatio(
        aspectRatio: aspect.clamp(0.5, 3.0),
        child: Container(
          color: Theme.of(context).colorScheme.surfaceContainerHighest,
          child: Center(child: child),
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
