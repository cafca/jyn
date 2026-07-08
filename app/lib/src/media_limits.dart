/// Per-kind size ceilings for media. Oversized attachments are rejected at the
/// composer (post time) and shown as a "too large" tile instead of being
/// fetched (render time), so one huge file can't dominate — or thrash — the
/// media cache. Audio and generic files are uncapped.
///
/// Keep in sync with `max_bytes_for_kind` in core/src/media/mod.rs.
library;

import 'rust/domain.dart';

const int kMaxPhotoBytes = 15 * 1024 * 1024; // 15 MB
const int kMaxVideoBytes = 200 * 1024 * 1024; // 200 MB

/// Human-readable forms for user-facing messages.
const String kMaxPhotoBytesLabel = '15 MB';
const String kMaxVideoBytesLabel = '200 MB';

/// The size ceiling for a kind, or null if the kind is uncapped.
int? maxBytesForKind(MediaKind kind) => switch (kind) {
  MediaKind.photo => kMaxPhotoBytes,
  MediaKind.video => kMaxVideoBytes,
  MediaKind.audio || MediaKind.file => null,
};

/// True if an attachment of this kind and size is over its ceiling.
bool exceedsLimit(MediaKind kind, int byteLen) {
  final cap = maxBytesForKind(kind);
  return cap != null && byteLen > cap;
}

/// Classifies a freshly picked file by extension, mirroring the Rust core's
/// `classify`, so the composer can size-check it before it has a [MediaKind].
MediaKind kindForPath(String path) {
  final slash = path.lastIndexOf(RegExp(r'[/\\]'));
  final dot = path.lastIndexOf('.');
  final ext = (dot > slash && dot != -1)
      ? path.substring(dot + 1).toLowerCase()
      : '';
  const photo = {'png', 'jpg', 'jpeg', 'gif', 'webp'};
  const audio = {'wav', 'mp3', 'flac', 'ogg', 'm4a'};
  const video = {'mp4', 'mov', 'webm', 'mkv', 'avi'};
  if (photo.contains(ext)) return MediaKind.photo;
  if (audio.contains(ext)) return MediaKind.audio;
  if (video.contains(ext)) return MediaKind.video;
  return MediaKind.file;
}

/// A short human label for a kind's limit, e.g. "15 MB". Null when uncapped.
String? limitLabelForKind(MediaKind kind) {
  final cap = maxBytesForKind(kind);
  if (cap == null) return null;
  return '${cap ~/ (1024 * 1024)} MB';
}
