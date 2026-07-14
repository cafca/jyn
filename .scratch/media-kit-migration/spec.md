# Spec — media_kit as the single media engine

**Status:** ready-for-agent

Governed by ADR 0018. Sequenced **first**; must be verified on macOS before the
Linux desktop-builds effort starts.

## Problem Statement

As someone posting to the river, I can attach and post a video or audio file in
a format the composer accepts — including `.mkv`, `.webm`, `.avi`, `.ogg`, and
`.flac` — but when I or a friend open the post, playback silently fails. The app
accepted media it cannot actually play, because the current video player defers
to the operating system's native decoder, which doesn't understand those
formats. Separately, the app cannot run on Linux at all, because the current
audio and video players have no Linux support.

## Solution

Every platform plays media through one engine (libmpv, via media_kit) that
decodes all the formats the composer accepts. Playback matches ingestion:
anything you can post, you and your friends can play back — the same on every
platform. Recording a voice note is unchanged.

## User Stories

1. As a poster, I want a video I attach in any accepted container (`mp4`, `mov`,
   `webm`, `mkv`, `avi`) to play back for me, so that I trust what I posted is
   viewable.
2. As a friend viewing the river, I want a post's video to play inline
   regardless of its container, so that I never hit a post I can't watch.
3. As a poster, I want an audio attachment in any accepted format (`wav`, `mp3`,
   `flac`, `ogg`, `m4a`) to play, so that my audio isn't silently dead.
4. As a listener, I want a voice note to play with its waveform and transport
   controls exactly as before, so that the experience is unchanged by the engine
   swap.
5. As a listener, I want to play, pause, and seek within a voice note, so that I
   can navigate a longer recording.
6. As a viewer, I want to play, pause, and seek within a video, so that I control
   playback.
7. As a poster, I want to still record a voice note in-app, so that the capture
   path is unaffected by the playback change.
8. As a macOS user on the already-shipping app, I want media that played before
   to keep playing after the change, so that the migration is a strict
   improvement with no regression.
9. As a viewer, I want a video that fails to decode to surface as a clear
   inert/error state rather than crashing the app, so that one bad file doesn't
   take down the river.
10. As a maintainer, I want a single media code path across platforms, so that
    future ports (Linux, Windows, iOS, Android) inherit working media for free.

## Implementation Decisions

- Replace the audio-playback and video-playback plugins with a single
  media_kit-based engine on every platform. Retire the previous `just_audio`
  and `video_player` dependencies. Keep the existing recording plugin — it is
  the voice-note capture path and media_kit does not record.
- Initialize the media engine once at application startup.
- The voice-note player keeps its externally-supplied waveform (the waveform
  travels in the post and renders before audio arrives) and its play/pause/seek
  controls; only the underlying playback source changes to a media_kit player.
- The video attachment renders through media_kit's video surface and its
  controller.
- libmpv is bundled on every platform (per ADR 0018). The core's media
  classification and blob storage are unchanged — media remains untranscoded,
  stored and replicated as-is.
- No change to the accepted-format list or the per-kind size limits.

## Testing Decisions

- A good test asserts external behavior — the widget builds, exposes play /
  pause / seek, and reflects a playing/paused state — not the plugin's
  internals.
- Widget tests cover the voice-note player and the video attachment: each builds
  with a media_kit controller and wires its transport controls. Prior art: the
  existing widget tests in the app test suite.
- Actual decode is verified manually on a macOS **release** build: record and
  play a voice note, and play at least one `mp4` and one `webm`/`mkv` video.
  Confirm no regression against the prior players and note the app-size change
  from bundling libmpv.

## Out of Scope

- Linux packaging and distribution — covered by the linux-desktop-builds spec,
  for which this migration is a prerequisite.
- iOS/Android media — later ports.
- Transcoding-on-ingest in the core (explicitly rejected in ADR 0018).
- Any refactor of the recording/capture path.

## Further Notes

- This is the higher-risk of the two efforts precisely because it touches the
  already-shipping macOS media path; hence the manual macOS verification gate
  before Linux work begins.
- `record`'s Linux capture backend needs its own verification, tracked with the
  Linux effort rather than here.
