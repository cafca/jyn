# media_kit (libmpv) is the single media engine on every platform

**Status:** accepted

The core stores media **untranscoded**: `classify` keys off the file extension
and blobs are materialized back as-is (`core/src/media/mod.rs`), and the
composer accepts `mkv`/`webm`/`avi` video and `ogg`/`flac` audio
(`app/lib/src/media_limits.dart`, mirrored in the core). The current player,
`video_player`, defers to the OS-native engine — AVFoundation on Apple — which
cannot decode `mkv`/`webm`/`avi`. So today the app **accepts media it then
silently fails to play**. Separately, `just_audio` and `video_player` have no
Linux/Windows support, which blocks every desktop port beyond macOS.

We will replace `just_audio` and `video_player` with **`media_kit`
(libmpv/FFmpeg) on every platform**. libmpv decodes essentially all the
containers and codecs the composer accepts, so playback finally matches
ingestion, and one media code path covers macOS, Linux, Windows, and (later)
iOS/Android. `record` stays — it is the capture path and media_kit does not
record. `voice_note_player` and `video_attachment` move to a media_kit
`Player`/`Video`.

## Considered Options

- **(A) media_kit everywhere** — chosen. One decode path with broad codec
  support; fixes the accept-but-can't-play gap on macOS and unblocks desktop.
- **(B) Confine media_kit to Linux, keep native players on Apple** — rejected.
  Leaves the macOS `mkv`/`webm`/`avi` gap unfixed and keeps two media code
  paths to maintain.
- **(C) Transcode-on-ingest in the core so `video_player` suffices** —
  rejected. Pulls FFmpeg into the core, is lossy, re-encodes every upload, and
  still doesn't solve the desktop-plugin gap.

## Consequences

- libmpv is bundled on **every** platform, so the macOS app grows in size and
  its already-shipping media path must be **re-verified** as part of the swap
  (this is a validation task, not a reason to avoid the change).
- `record`'s Linux capture backend is less battle-tested than its playback
  counterpart and needs verifying when the Linux port lands.
- media_kit_video pulls in `wakelock_plus` → `package_info_plus`, whose macOS
  Swift Package Manager manifest declares a 10.14 floor below FlutterFramework's
  10.15, which breaks the build under Flutter's SPM. The app therefore builds on
  the CocoaPods path (Flutter SPM disabled) — CI's default — until the plugin
  ecosystem's SPM manifests catch up.
