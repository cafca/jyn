import 'dart:io';

import 'package:flutter/material.dart' hide Visibility;
import 'package:video_player/video_player.dart';

import '../rust/domain.dart';
import '../theme/tokens.dart';

/// Inline video: a poster-less tap-to-play frame once the blob is local,
/// a downloading placeholder before. Fills whatever frame the parent
/// provides (the feed's 4:5 tile), cover-cropping the video.
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
    final controller = _controller;

    if (controller == null || !controller.value.isInitialized) {
      return Container(
        color: JynColors.cardGrey,
        child: Center(
          child: widget.path == null
              ? const Icon(Icons.downloading, color: JynColors.muted)
              : const CircularProgressIndicator(color: JynColors.mid),
        ),
      );
    }

    final videoSize = controller.value.size;
    return Stack(
      alignment: Alignment.center,
      fit: StackFit.expand,
      children: [
        // Cover-crop the video into the parent's frame.
        FittedBox(
          fit: BoxFit.cover,
          clipBehavior: Clip.hardEdge,
          child: SizedBox(
            width: videoSize.width,
            height: videoSize.height,
            child: VideoPlayer(controller),
          ),
        ),
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
    );
  }
}
