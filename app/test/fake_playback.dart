import 'dart:async';

import 'package:jyn/src/widgets/media_playback.dart';

/// A fully in-memory [MediaPlayback] so the media widgets' transport wiring can
/// be exercised without native libmpv. Records the calls the widget makes and
/// lets the test push playback state back through the streams.
class FakePlayback implements MediaPlayback {
  final _playing = StreamController<bool>.broadcast();
  final _completed = StreamController<bool>.broadcast();
  final _position = StreamController<Duration>.broadcast();

  final List<String> calls = [];
  String? openedPath;
  Duration? seekedTo;
  Duration? overrideDuration;

  void emitPlaying(bool value) => _playing.add(value);
  void emitCompleted(bool value) => _completed.add(value);
  void emitPosition(Duration value) => _position.add(value);

  @override
  Future<void> open(String path) async {
    openedPath = path;
    calls.add('open');
  }

  @override
  Future<void> play() async => calls.add('play');

  @override
  Future<void> pause() async => calls.add('pause');

  @override
  Future<void> seek(Duration position) async {
    seekedTo = position;
    calls.add('seek');
  }

  @override
  Duration? get duration => overrideDuration;

  @override
  Stream<bool> get playingStream => _playing.stream;

  @override
  Stream<bool> get completedStream => _completed.stream;

  @override
  Stream<Duration> get positionStream => _position.stream;

  @override
  Future<void> dispose() async {
    calls.add('dispose');
    await _playing.close();
    await _completed.close();
    await _position.close();
  }
}
