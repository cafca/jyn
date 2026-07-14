import 'dart:async';

import 'package:flutter/material.dart' hide Visibility;

import '../theme/tokens.dart';
import 'media_playback.dart';

/// A voice note: the waveform travels in the post and renders before the
/// audio blob arrives; playback unlocks once the file is local. Drives both
/// the composer's draft (a local `.wav`) and posted attachments (a blob from
/// the media cache), so it takes the raw pieces rather than a whole
/// [MediaAttachment].
class VoiceNotePlayer extends StatefulWidget {
  const VoiceNotePlayer({
    super.key,
    required this.waveform,
    required this.durationMs,
    this.path,
    this.playbackFactory = createMediaKitPlayback,
  });

  /// Peak buckets (0..=255), rendered before the blob arrives.
  final List<int>? waveform;
  final int? durationMs;

  /// The local file, or null while the blob is still being fetched.
  final String? path;

  /// Injectable so tests can drive playback with a fake (the real engine needs
  /// native libmpv). Production uses the media_kit-backed default.
  final MediaPlaybackFactory playbackFactory;

  @override
  State<VoiceNotePlayer> createState() => _VoiceNotePlayerState();
}

class _VoiceNotePlayerState extends State<VoiceNotePlayer> {
  MediaPlayback? _playback;
  final List<StreamSubscription<dynamic>> _subs = [];
  bool _playing = false;
  bool _completed = false;
  Duration _position = Duration.zero;

  @override
  void dispose() {
    for (final sub in _subs) {
      sub.cancel();
    }
    _playback?.dispose();
    super.dispose();
  }

  /// Lazily opens the audio file — shared by play and seek, so tapping into
  /// the waveform works before the note has ever been played. Returns null if
  /// the blob isn't local yet or the file can't be opened. libmpv probes the
  /// container from content, so the content-addressed path is fed directly.
  Future<MediaPlayback?> _ensurePlayback() async {
    final existing = _playback;
    if (existing != null) return existing;
    final path = widget.path;
    if (path == null) return null;
    final playback = widget.playbackFactory();
    try {
      await playback.open(path);
    } catch (error) {
      await playback.dispose();
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('could not open voice note: $error')),
        );
      }
      return null;
    }
    if (!mounted) {
      await playback.dispose();
      return null;
    }
    _subs.add(
      playback.playingStream.listen((playing) {
        if (mounted) setState(() => _playing = playing);
      }),
    );
    _subs.add(
      playback.completedStream.listen((completed) {
        if (mounted) setState(() => _completed = completed);
      }),
    );
    _subs.add(
      playback.positionStream.listen((position) {
        if (mounted) setState(() => _position = position);
      }),
    );
    setState(() => _playback = playback);
    return playback;
  }

  Future<void> _toggle() async {
    final playback = await _ensurePlayback();
    if (playback == null) return;
    // A finished note replays from the top rather than resuming at the end.
    if (_completed) {
      setState(() => _completed = false);
      await playback.seek(Duration.zero);
      await playback.play();
    } else if (_playing) {
      await playback.pause();
    } else {
      await playback.play();
    }
  }

  /// Jumps to [fraction] (0..1) of the note. Keeps playing if it was, stays
  /// paused otherwise — a completed note becomes seekable again.
  Future<void> _seek(double fraction) async {
    final playback = await _ensurePlayback();
    if (playback == null) return;
    final total = _totalDuration(playback);
    if (total == null) return;
    if (_completed) setState(() => _completed = false);
    await playback.seek(total * fraction.clamp(0.0, 1.0));
  }

  /// The note's length: the player's own duration once loaded, else the
  /// summary that travelled with the post.
  Duration? _totalDuration(MediaPlayback? playback) {
    final loaded = playback?.duration;
    if (loaded != null) return loaded;
    final ms = widget.durationMs;
    return ms != null ? Duration(milliseconds: ms) : null;
  }

  @override
  Widget build(BuildContext context) {
    final waveform = widget.waveform ?? const <int>[];
    final durationMs = widget.durationMs;
    final seconds = durationMs != null ? (durationMs / 1000).round() : null;

    final fetching = widget.path == null;
    // A finished note shows as paused so the button invites a replay.
    final playing = _playing && !_completed;
    final total = _totalDuration(_playback);
    final progress = (total != null && total.inMilliseconds > 0)
        ? _position.inMilliseconds / total.inMilliseconds
        : 0.0;

    // The design's audio card: green-tinted ground, 44px leaf play circle,
    // a waveform fading teal→mist left to right, mono duration.
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: JynColors.cardGreen,
        borderRadius: BorderRadius.circular(JynRadii.card),
      ),
      child: Row(
        children: [
          MouseRegion(
            cursor: fetching
                ? SystemMouseCursors.basic
                : SystemMouseCursors.click,
            child: GestureDetector(
              onTap: fetching ? null : _toggle,
              child: Container(
                width: 44,
                height: 44,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  color: fetching ? JynColors.muted : JynColors.leaf,
                ),
                child: Icon(
                  fetching
                      ? Icons.downloading
                      : playing
                      ? Icons.pause
                      : Icons.play_arrow,
                  size: 22,
                  color: Colors.white,
                ),
              ),
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: SizedBox(
              height: 36,
              child: LayoutBuilder(
                builder: (context, constraints) => GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onTapDown: (details) =>
                      _seek(details.localPosition.dx / constraints.maxWidth),
                  onHorizontalDragUpdate: (details) =>
                      _seek(details.localPosition.dx / constraints.maxWidth),
                  child: CustomPaint(
                    painter: _WaveformPainter(
                      peaks: waveform,
                      progress: progress.clamp(0.0, 1.0),
                    ),
                  ),
                ),
              ),
            ),
          ),
          if (seconds != null)
            Padding(
              padding: const EdgeInsets.only(left: 12),
              child: Text(
                '${seconds ~/ 60}:${(seconds % 60).toString().padLeft(2, '0')}',
                style: const TextStyle(
                  fontFamily: JynType.mono,
                  fontSize: 12,
                  color: JynColors.audioDuration,
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// Peak buckets (0..=255) as centered 3px bars on a 5.5px pitch (the
/// design's spacing), resampled to the available width. Bar color fades
/// teal→mist left to right per the design tokens; bars right of
/// [progress] render dimmed — a playback fill.
class _WaveformPainter extends CustomPainter {
  const _WaveformPainter({required this.peaks, required this.progress});

  final List<int> peaks;
  final double progress;

  static const _pitch = 5.5; // 3px bar + 2.5px gap

  @override
  void paint(Canvas canvas, Size size) {
    if (peaks.isEmpty) return;
    final bars = (size.width / _pitch).floor().clamp(1, peaks.length);
    final step = size.width / bars;
    final playedWidth = size.width * progress;
    for (var index = 0; index < bars; index++) {
      final peak = peaks[(index * peaks.length) ~/ bars];
      final t = bars == 1 ? 0.0 : index / (bars - 1);
      final color = _fade(t);
      final paint = Paint()
        ..color = x(index, step) <= playedWidth
            ? color
            : color.withValues(alpha: 0.45)
        ..strokeWidth = 3
        ..strokeCap = StrokeCap.round;
      final magnitude = (peak / 255) * (size.height / 2 - 2);
      final half = magnitude.clamp(2.0, size.height / 2);
      canvas.drawLine(
        Offset(x(index, step), size.height / 2 - half),
        Offset(x(index, step), size.height / 2 + half),
        paint,
      );
    }
  }

  double x(int index, double step) => step * index + step / 2;

  /// Interpolates across the design's four waveform stops.
  Color _fade(double t) {
    final stops = JynColors.waveform;
    final scaled = t * (stops.length - 1);
    final low = scaled.floor().clamp(0, stops.length - 2);
    return Color.lerp(stops[low], stops[low + 1], scaled - low)!;
  }

  @override
  bool shouldRepaint(_WaveformPainter oldDelegate) =>
      oldDelegate.peaks != peaks || oldDelegate.progress != progress;
}
