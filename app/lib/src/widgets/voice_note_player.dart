import 'package:flutter/material.dart' hide Visibility;
import 'package:just_audio/just_audio.dart';

import '../rust/domain.dart';

/// A voice note: the waveform travels in the post and renders before the
/// audio blob arrives; playback unlocks once the file is local.
class VoiceNotePlayer extends StatefulWidget {
  const VoiceNotePlayer({super.key, required this.attachment, this.path});

  final MediaAttachment attachment;
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

  Future<void> _toggle() async {
    final path = widget.path;
    if (path == null) return;
    var player = _player;
    if (player == null) {
      player = AudioPlayer();
      await player.setFilePath(path);
      setState(() => _player = player);
    }
    if (player.playing) {
      await player.pause();
    } else {
      if (player.processingState == ProcessingState.completed) {
        await player.seek(Duration.zero);
      }
      await player.play();
    }
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final waveform = widget.attachment.waveform ?? const <int>[];
    final durationMs = widget.attachment.durationMs;
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
              final playing = snapshot.data?.playing ?? false;
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
            child: CustomPaint(
              painter: _WaveformPainter(
                peaks: waveform,
                color: scheme.primary,
              ),
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

/// Peak buckets (0..=255) as centered vertical bars.
class _WaveformPainter extends CustomPainter {
  const _WaveformPainter({required this.peaks, required this.color});

  final List<int> peaks;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    if (peaks.isEmpty) return;
    final paint = Paint()
      ..color = color
      ..strokeWidth = 3
      ..strokeCap = StrokeCap.round;
    final step = size.width / peaks.length;
    for (final (index, peak) in peaks.indexed) {
      final x = step * index + step / 2;
      final magnitude = (peak / 255) * (size.height / 2 - 2);
      final half = magnitude.clamp(1.0, size.height / 2);
      canvas.drawLine(
        Offset(x, size.height / 2 - half),
        Offset(x, size.height / 2 + half),
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(_WaveformPainter oldDelegate) =>
      oldDelegate.peaks != peaks || oldDelegate.color != color;
}
