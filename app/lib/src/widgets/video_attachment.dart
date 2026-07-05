import 'dart:io';

import 'package:flutter/material.dart' hide Visibility;
import 'package:video_player/video_player.dart';

import '../rust/domain.dart';

/// Inline video: a poster-less tap-to-play frame once the blob is local,
/// a downloading placeholder before.
class VideoAttachment extends StatefulWidget {
  const VideoAttachment({
    super.key,
    required this.attachment,
    required this.path,
    required this.playerKey,
  });

  final MediaAttachment attachment;
  final String? path;

  /// Stable identity (post + blob) so list rebuilds don't restart playback.
  final String playerKey;

  @override
  State<VideoAttachment> createState() => _VideoAttachmentState();
}

class _VideoAttachmentState extends State<VideoAttachment> {
  VideoPlayerController? _controller;
  String? _initializedFor;

  @override
  void didUpdateWidget(VideoAttachment oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.path != null && _initializedFor != widget.path) {
      _initController();
    }
  }

  @override
  void initState() {
    super.initState();
    if (widget.path != null) _initController();
  }

  Future<void> _initController() async {
    final path = widget.path;
    if (path == null) return;
    _initializedFor = path;
    final controller = VideoPlayerController.file(File(path));
    await controller.initialize();
    await controller.setLooping(false);
    if (!mounted) {
      await controller.dispose();
      return;
    }
    setState(() => _controller = controller);
  }

  @override
  void dispose() {
    _controller?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final controller = _controller;
    final aspect = _aspect();

    if (controller == null || !controller.value.isInitialized) {
      return ClipRRect(
        borderRadius: BorderRadius.circular(8),
        child: AspectRatio(
          aspectRatio: aspect,
          child: Container(
            color: scheme.surfaceContainerHighest,
            child: Center(
              child: widget.path == null
                  ? const Icon(Icons.downloading)
                  : const CircularProgressIndicator(),
            ),
          ),
        ),
      );
    }

    return ClipRRect(
      borderRadius: BorderRadius.circular(8),
      child: AspectRatio(
        aspectRatio: controller.value.aspectRatio,
        child: Stack(
          alignment: Alignment.center,
          children: [
            VideoPlayer(controller),
            ValueListenableBuilder(
              valueListenable: controller,
              builder: (context, value, _) {
                if (value.isPlaying) {
                  return GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: controller.pause,
                    child: const SizedBox.expand(),
                  );
                }
                return IconButton(
                  iconSize: 56,
                  onPressed: controller.play,
                  icon: Icon(
                    Icons.play_circle_filled,
                    color: Colors.white.withValues(alpha: 0.9),
                    shadows: const [Shadow(blurRadius: 8)],
                  ),
                );
              },
            ),
          ],
        ),
      ),
    );
  }

  double _aspect() {
    final width = widget.attachment.width;
    final height = widget.attachment.height;
    if (width != null && height != null && height > 0) {
      return (width / height).clamp(0.5, 3.0);
    }
    return 16 / 9;
  }
}
