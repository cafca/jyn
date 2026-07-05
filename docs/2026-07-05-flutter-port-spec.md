# Spec: Port jyn's UI from Bevy to Flutter

Agreed 2026-07-05. Replaces the Bevy/egui interface with a Flutter app; the
p2panda core stays in Rust. Target platforms: macOS first, then iOS, later
Windows/Linux/Android.

## Architecture

- **Rust core + Flutter UI, bound with `flutter_rust_bridge` v2**, in-process.
  No Swift/Kotlin middle layer, no daemon.
- Cargokit build integration so `flutter run`/`flutter build` compiles the
  Rust crate automatically per platform.
- The architecture must not preclude any of the five target platforms
  (and FRB doesn't).

## Repo layout

- Same repo, restructured: **`core/`** (the existing crate minus UI) and
  **`app/`** (new Flutter app).
- Bevy, egui, and the render code (`src/ui/`, `src/render/`,
  `src/plugin.rs`, card effects, water shader) are **deleted in the port
  branch**. Git history is the archive.

## FFI surface

- **One async function per command** (`publishPost`, `deletePost`,
  `acceptFriendRequest`, …) returning `Result` — awaitable in Dart, throws
  on failure. Internally a thin façade over the existing command loop.
- **One event stream** (`events() → Stream<NetworkEvent>`) for everything
  push-driven: posts, sync, friends, diagnostics, expiry. FRB mirrors the
  enums as sealed Dart classes.

## Flutter app

- **Riverpod** for state; the event stream feeds a `StreamProvider`,
  screens derive from provider composition.
- **Fidelity bar: functional parity, plain visuals.** All features work —
  composer (visibility, lifetime), river feed with countdowns,
  edit/delete/promote/let-go, named hearts, friend codes and requests,
  keeps, profile, onboarding, diagnostics, voice notes, photos, video.
  Stock widgets plus a light theme pass; custom painting only for the
  waveform. No shaders, no card effects.

## Media

- **Flutter owns capture and playback:** `record` (voice notes),
  `just_audio` (playback), `video_player` (inline video everywhere — ffmpeg
  dependency gone), native `Image` for photos.
- **Rust keeps** blob import/export and **waveform peak generation** at
  cast time, so waveform-travels-in-the-post is unchanged.
- `cpal`, `rodio`, `hound` (playback path), ffmpeg handling, `rfd`,
  `arboard`, `open` all leave the core; Flutter plugins replace the last
  three (file picker, clipboard, url_launcher).

## Data

- **Clean slate.** New data dir, fresh identity, zero migration code.
  On-disk formats are free to change during the port.

## Testing

- Existing Rust tests keep passing against `core/`.
- Dart tests only where logic lives: providers (river ordering,
  countdown/expiry, keep-lease behavior), command-façade error handling.
  No widget/integration tests until the redesign.

## Known small tasks

- macOS entitlements for a sandboxed Flutter app: network client/server
  (p2panda), microphone.
- Two-instance local testing (`JYN_DATA_DIR`-style override) must survive
  in the Flutter world for dev workflows.
