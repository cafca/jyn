import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart' hide Visibility;
import 'package:record/record.dart';

import '../rust/api/commands.dart';
import '../rust/api/media.dart' as rust_media;

/// The composer's mic button: tap to record a voice note (WAV), tap again to
/// stop. The core reduces the file to duration + waveform peaks so they can
/// travel inside the post. A missing or denied microphone disables the
/// button — never a crash.
class VoiceRecorderButton extends StatefulWidget {
  const VoiceRecorderButton({super.key, required this.onRecorded});

  final ValueChanged<MediaDraftInput> onRecorded;

  @override
  State<VoiceRecorderButton> createState() => _VoiceRecorderButtonState();
}

class _VoiceRecorderButtonState extends State<VoiceRecorderButton> {
  final _recorder = AudioRecorder();
  bool _recording = false;
  String? _micError;

  /// Wall-clock length of the take in progress, surfaced next to the button.
  final _elapsed = Stopwatch();
  Timer? _ticker;

  @override
  void dispose() {
    _ticker?.cancel();
    _recorder.dispose();
    super.dispose();
  }

  Future<void> _toggle() async {
    if (_recording) {
      _ticker?.cancel();
      _elapsed.stop();
      final path = await _recorder.stop();
      setState(() => _recording = false);
      if (path == null) return;
      try {
        final summary = await rust_media.voiceNoteSummary(wavPath: path);
        widget.onRecorded(
          MediaDraftInput(
            path: path,
            durationMs: summary.durationMs,
            waveform: summary.waveform,
          ),
        );
      } catch (error) {
        if (mounted) {
          ScaffoldMessenger.of(
            context,
          ).showSnackBar(SnackBar(content: Text('voice note failed: $error')));
        }
      }
      return;
    }

    try {
      if (!await _recorder.hasPermission()) {
        setState(() => _micError = 'microphone access denied');
        return;
      }
      final dir = await Directory.systemTemp.createTemp('jyn-recording');
      await _recorder.start(
        const RecordConfig(
          encoder: AudioEncoder.wav,
          sampleRate: 44100,
          numChannels: 1,
        ),
        path: '${dir.path}/voice-note.wav',
      );
      _elapsed
        ..reset()
        ..start();
      _ticker = Timer.periodic(const Duration(seconds: 1), (_) {
        if (mounted) setState(() {});
      });
      setState(() {
        _recording = true;
        _micError = null;
      });
    } catch (error) {
      setState(() => _micError = error.toString());
    }
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final button = IconButton(
      tooltip:
          _micError ?? (_recording ? 'stop recording' : 'record a voice note'),
      onPressed: _micError != null && !_recording ? null : _toggle,
      isSelected: _recording,
      icon: Icon(
        _micError != null
            ? Icons.mic_off
            : _recording
            ? Icons.stop_circle
            : Icons.mic_none,
        color: _recording ? scheme.error : null,
      ),
    );
    if (!_recording) return button;

    final secs = _elapsed.elapsed.inSeconds;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          '${secs ~/ 60}:${(secs % 60).toString().padLeft(2, '0')}',
          style: Theme.of(context).textTheme.labelMedium?.copyWith(
            color: scheme.error,
            fontFeatures: const [FontFeature.tabularFigures()],
          ),
        ),
        button,
      ],
    );
  }
}
