import 'dart:async';

import 'package:flutter/material.dart' hide Visibility;
import 'package:media_kit_video/media_kit_video.dart';

import '../rust/domain.dart';
import '../theme/tokens.dart';
import 'media_playback.dart';

/// Inline video: a poster-less tap-to-play frame once the blob is local,
/// a downloading placeholder before. Fills whatever frame the parent
/// provides (the feed's 4:5 tile), cover-cropping the video.
class VideoAttachment extends StatefulWidget {
  const VideoAttachment({
    super.key,
    required this.attachment,
    required this.path,
    required this.playerKey,
    this.playbackFactory = createMediaKitPlayback,
  });

  final MediaAttachment attachment;
  final String? path;

  /// Stable identity (post + blob) so list rebuilds don't restart playback.
  final String playerKey;

  /// Injectable so tests can drive the pre-render branches with a fake (the
  /// video surface needs native libmpv). Production uses the media_kit default.
  final MediaPlaybackFactory playbackFactory;

  @override
  State<VideoAttachment> createState() => _VideoAttachmentState();
}

class _VideoAttachmentState extends State<VideoAttachment> {
  MediaPlayback? _playback;
  VideoController? _controller;
  String? _initializedFor;
  final List<StreamSubscription<dynamic>> _subs = [];
  bool _ready = false;
  bool _playing = false;

  @override
  void initState() {
    super.initState();
    if (widget.path != null) _init();
  }

  @override
  void didUpdateWidget(VideoAttachment oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.path != null && _initializedFor != widget.path) _init();
  }

  Future<void> _init() async {
    final path = widget.path;
    if (path == null) return;
    // Tear down any prior player (the blob path changed).
    await _teardown();
    _initializedFor = path;
    final playback = widget.playbackFactory();
    _playback = playback;
    // The video render surface needs the concrete media_kit player; a fake
    // (tests) drives only the pre-render placeholder branches.
    if (playback is MediaKitPlayback) {
      _controller = VideoController(playback.player);
      _subs.add(
        playback.player.stream.width.listen((width) {
          if (mounted && width != null && width > 0 && !_ready) {
            setState(() => _ready = true);
          }
        }),
      );
    }
    _subs.add(
      playback.playingStream.listen((playing) {
        if (mounted) setState(() => _playing = playing);
      }),
    );
    try {
      await playback.open(path);
    } catch (_) {
      // A file libmpv can't open leaves the loading placeholder up rather than
      // crashing the feed.
    }
  }

  Future<void> _teardown() async {
    for (final sub in _subs) {
      sub.cancel();
    }
    _subs.clear();
    await _playback?.dispose();
    _playback = null;
    _controller = null;
    if (mounted) {
      setState(() {
        _ready = false;
        _playing = false;
      });
    } else {
      _ready = false;
      _playing = false;
    }
  }

  @override
  void dispose() {
    for (final sub in _subs) {
      sub.cancel();
    }
    _playback?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final controller = _controller;

    if (!_ready || controller == null) {
      return Container(
        color: JynColors.cardGrey,
        child: Center(
          child: widget.path == null
              ? const Icon(Icons.downloading, color: JynColors.muted)
              : const CircularProgressIndicator(color: JynColors.mid),
        ),
      );
    }

    return Stack(
      alignment: Alignment.center,
      fit: StackFit.expand,
      children: [
        // media_kit's Video cover-crops into the parent frame itself.
        Video(
          controller: controller,
          fit: BoxFit.cover,
          controls: NoVideoControls,
        ),
        if (_playing)
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () => _playback?.pause(),
            child: const SizedBox.expand(),
          )
        else
          IconButton(
            iconSize: 56,
            onPressed: () => _playback?.play(),
            icon: Icon(
              Icons.play_circle_filled,
              color: Colors.white.withValues(alpha: 0.9),
              shadows: const [Shadow(blurRadius: 8)],
            ),
          ),
      ],
    );
  }
}
