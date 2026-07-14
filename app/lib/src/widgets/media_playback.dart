import 'package:media_kit/media_kit.dart';

/// A thin seam over media_kit's [Player]. The media widgets depend on this
/// interface rather than the concrete player so a fake can drive them under
/// `flutter test` — the real player needs native libmpv, which the Dart test
/// VM doesn't load. Production code uses [MediaKitPlayback] via
/// [createMediaKitPlayback].
abstract class MediaPlayback {
  /// Loads [path] without starting playback.
  Future<void> open(String path);
  Future<void> play();
  Future<void> pause();
  Future<void> seek(Duration position);

  /// The decoded media's total length, or null before it is known.
  Duration? get duration;

  Stream<bool> get playingStream;
  Stream<bool> get completedStream;
  Stream<Duration> get positionStream;

  Future<void> dispose();
}

/// Creates a fresh [MediaPlayback]. Widgets take one of these so tests can
/// substitute a fake; production passes [createMediaKitPlayback].
typedef MediaPlaybackFactory = MediaPlayback Function();

/// Production factory: a media_kit-backed player.
MediaPlayback createMediaKitPlayback() => MediaKitPlayback();

/// media_kit-backed [MediaPlayback]. Exposes the underlying [Player] so the
/// video widget can attach a [VideoController]; audio-only callers ignore it.
class MediaKitPlayback implements MediaPlayback {
  MediaKitPlayback() : player = Player();

  final Player player;

  @override
  Future<void> open(String path) => player.open(Media(path), play: false);

  @override
  Future<void> play() => player.play();

  @override
  Future<void> pause() => player.pause();

  @override
  Future<void> seek(Duration position) => player.seek(position);

  @override
  Duration? get duration {
    final value = player.state.duration;
    // media_kit reports Duration.zero before a track is decoded; treat that as
    // "unknown" so callers fall back to the summary that travelled in the post.
    return value == Duration.zero ? null : value;
  }

  @override
  Stream<bool> get playingStream => player.stream.playing;

  @override
  Stream<bool> get completedStream => player.stream.completed;

  @override
  Stream<Duration> get positionStream => player.stream.position;

  @override
  Future<void> dispose() => player.dispose();
}
