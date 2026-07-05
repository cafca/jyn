import 'dart:io';

import 'package:flutter/material.dart' hide Visibility;
import 'package:just_audio/just_audio.dart';

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
    required this.mime,
    this.path,
  });

  /// Peak buckets (0..=255), rendered before the blob arrives.
  final List<int>? waveform;
  final int? durationMs;

  /// The audio container's mime type, used to give the player a file with a
  /// recognisable extension (see [_playablePath]).
  final String mime;

  /// The local file, or null while the blob is still being fetched.
  final String? path;

  @override
  State<VoiceNotePlayer> createState() => _VoiceNotePlayerState();
}

class _VoiceNotePlayerState extends State<VoiceNotePlayer> {
  AudioPlayer? _player;

  @override
  void dispose() {
    _player?.dispose();
    super.dispose();
  }

  /// Lazily opens the audio file — shared by play and seek, so tapping into
  /// the waveform works before the note has ever been played. Returns null if
  /// the blob isn't local yet or the file can't be opened.
  Future<AudioPlayer?> _ensurePlayer() async {
    final existing = _player;
    if (existing != null) return existing;
    final path = widget.path;
    if (path == null) return null;
    final player = AudioPlayer();
    try {
      await player.setFilePath(await _playablePath(path, widget.mime));
    } catch (error) {
      await player.dispose();
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('could not open voice note: $error')),
        );
      }
      return null;
    }
    if (!mounted) {
      await player.dispose();
      return null;
    }
    setState(() => _player = player);
    return player;
  }

  Future<void> _toggle() async {
    final player = await _ensurePlayer();
    if (player == null) return;
    // just_audio keeps `playing == true` after a track ends (only
    // processingState flips to completed), so check completion first: a tap
    // there replays from the top rather than pausing an already-stopped note.
    if (player.processingState == ProcessingState.completed) {
      await player.seek(Duration.zero);
      await player.play();
    } else if (player.playing) {
      await player.pause();
    } else {
      await player.play();
    }
  }

  /// Jumps to [fraction] (0..1) of the note. Keeps playing if it was, stays
  /// paused otherwise — a completed note becomes seekable again.
  Future<void> _seek(double fraction) async {
    final player = await _ensurePlayer();
    if (player == null) return;
    final total = _totalDuration(player);
    if (total == null) return;
    await player.seek(total * fraction.clamp(0.0, 1.0));
  }

  /// The note's length: the player's own duration once loaded, else the
  /// summary that travelled with the post.
  Duration? _totalDuration(AudioPlayer? player) {
    final loaded = player?.duration;
    if (loaded != null) return loaded;
    final ms = widget.durationMs;
    return ms != null ? Duration(milliseconds: ms) : null;
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final waveform = widget.waveform ?? const <int>[];
    final durationMs = widget.durationMs;
    final seconds = durationMs != null ? (durationMs / 1000).round() : null;

    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
      decoration: BoxDecoration(
        color: scheme.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          StreamBuilder<PlayerState>(
            stream: _player?.playerStateStream,
            builder: (context, snapshot) {
              final state = snapshot.data;
              // A finished note reports playing == true; show it as paused so
              // the button invites a replay.
              final playing =
                  (state?.playing ?? false) &&
                  state?.processingState != ProcessingState.completed;
              final fetching = widget.path == null;
              return IconButton(
                visualDensity: VisualDensity.compact,
                tooltip: fetching ? 'fetching audio…' : null,
                onPressed: fetching ? null : _toggle,
                icon: Icon(
                  fetching
                      ? Icons.downloading
                      : playing
                      ? Icons.pause_circle_filled
                      : Icons.play_circle_filled,
                  color: fetching ? scheme.outline : scheme.primary,
                ),
              );
            },
          ),
          SizedBox(
            width: 120,
            height: 28,
            child: StreamBuilder<Duration>(
              stream: _player?.positionStream,
              builder: (context, snapshot) {
                final position = snapshot.data ?? Duration.zero;
                final total = _totalDuration(_player);
                final progress = (total != null && total.inMilliseconds > 0)
                    ? position.inMilliseconds / total.inMilliseconds
                    : 0.0;
                return LayoutBuilder(
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
                        playedColor: scheme.primary,
                        unplayedColor: scheme.onSurfaceVariant.withValues(
                          alpha: 0.4,
                        ),
                      ),
                    ),
                  ),
                );
              },
            ),
          ),
          if (seconds != null)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8),
              child: Text(
                '${seconds ~/ 60}:${(seconds % 60).toString().padLeft(2, '0')}',
                style: Theme.of(context).textTheme.labelSmall,
              ),
            ),
        ],
      ),
    );
  }
}

/// just_audio infers the audio container from the file extension on Apple
/// platforms (AVFoundation), but the media cache is content-addressed with no
/// extension — feeding it directly throws `(-11828) Cannot Open`. Hand the
/// player a symlink that carries an extension derived from the mime type;
/// paths that already have the right one (drafts, `voice-note.wav`) pass
/// through untouched.
Future<String> _playablePath(String path, String mime) async {
  final ext = _audioExtension(mime);
  if (ext.isEmpty || path.toLowerCase().endsWith(ext)) return path;
  final dir = await Directory.systemTemp.createTemp('jyn-audio');
  final linkPath = '${dir.path}/voice-note$ext';
  try {
    await Link(linkPath).create(path);
  } on FileSystemException {
    // Symlinks can be unavailable (e.g. unprivileged Windows); copy instead.
    await File(path).copy(linkPath);
  }
  return linkPath;
}

String _audioExtension(String mime) => switch (mime) {
  'audio/wav' || 'audio/x-wav' || 'audio/wave' => '.wav',
  'audio/mpeg' || 'audio/mp3' => '.mp3',
  'audio/mp4' || 'audio/aac' || 'audio/x-m4a' => '.m4a',
  'audio/flac' || 'audio/x-flac' => '.flac',
  'audio/ogg' || 'audio/opus' => '.ogg',
  _ => '',
};

/// Peak buckets (0..=255) as centered vertical bars. Bars left of [progress]
/// (0..1) are drawn played, the rest dimmed — a playback fill.
class _WaveformPainter extends CustomPainter {
  const _WaveformPainter({
    required this.peaks,
    required this.progress,
    required this.playedColor,
    required this.unplayedColor,
  });

  final List<int> peaks;
  final double progress;
  final Color playedColor;
  final Color unplayedColor;

  @override
  void paint(Canvas canvas, Size size) {
    if (peaks.isEmpty) return;
    final played = Paint()
      ..color = playedColor
      ..strokeWidth = 3
      ..strokeCap = StrokeCap.round;
    final unplayed = Paint()
      ..color = unplayedColor
      ..strokeWidth = 3
      ..strokeCap = StrokeCap.round;
    final step = size.width / peaks.length;
    final playedWidth = size.width * progress;
    for (final (index, peak) in peaks.indexed) {
      final x = step * index + step / 2;
      final magnitude = (peak / 255) * (size.height / 2 - 2);
      final half = magnitude.clamp(1.0, size.height / 2);
      canvas.drawLine(
        Offset(x, size.height / 2 - half),
        Offset(x, size.height / 2 + half),
        x <= playedWidth ? played : unplayed,
      );
    }
  }

  @override
  bool shouldRepaint(_WaveformPainter oldDelegate) =>
      oldDelegate.peaks != peaks ||
      oldDelegate.progress != progress ||
      oldDelegate.playedColor != playedColor ||
      oldDelegate.unplayedColor != unplayedColor;
}
