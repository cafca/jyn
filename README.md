# jyn

A p2p board for a small circle of friends, built on p2panda. Posts flow down
one river; the only thing separating them is their author-chosen lifetime —
ephemeral posts drain away, permanent ones settle. The poster is sovereign,
not the platform: edits are marked, deletes reach every copy (kept ones
included), and nobody outside the friendship can see or erase anything.

Experimental. Expect breaking changes with any version update; on-disk
stores are wiped on schema mismatches (only `node.key` and `settings.json`
survive).

## Architecture

- `core/` — the Rust core: p2panda node, sync, stores, domain logic, and the
  headless runtime that derives UI state. Compiled into the app as a static
  library via [flutter_rust_bridge](https://github.com/fzyzcjy/flutter_rust_bridge).
- `app/` — the Flutter app (macOS first; iOS next; Windows/Linux/Android
  planned). Talks to the core through generated bindings: user actions are
  awaitable calls, state arrives as one event stream.

See `docs/2026-07-05-flutter-port-spec.md` for the port's decisions.

## Running

Requires Flutter (3.41+) and a Rust toolchain; `flutter build` compiles the
core automatically via cargokit.

```
cd app
flutter run -d macos
```

Two instances side by side for local testing:

```
JYN_DATA_DIR=/tmp/jyn-a flutter run -d macos
JYN_DATA_DIR=/tmp/jyn-b flutter run -d macos
```

Rust tests: `cargo test` (repo root). Dart tests: `flutter test` in `app/`.
After changing the API surface in `core/src/api/`, regenerate bindings with
`flutter_rust_bridge_codegen generate` in `app/`.

## Releases & updates

The macOS app updates itself via Sparkle: it checks the appcast on launch and
daily, and offers an "Check for Updates…" item in the app menu. Releases are
cut locally with `scripts/release.sh` (build → notarize → EdDSA-sign →
appcast → GitHub Release), driven by the repo-root `VERSION` file. See
[docs/2026-07-05-auto-updater.md](docs/2026-07-05-auto-updater.md) for the
one-time signing setup and the release runbook. iOS/Android update through
their app stores; Linux/Windows backends come with those ports.

## What it does

- **One post type.** Lifetime (ephemeral with a visible countdown, or
  permanent) is the only differentiating property. Authors can promote,
  let go, edit (marked) and delete (reaches kept copies).
- **Visibility per post:** public / circles / friends / only-you. Private
  posts never enter the replicated log at all. In v1 everything else
  replicates to friends only.
- **Consented friendship, nothing else.** Hand a `jyn-` friend code over
  any channel you trust, or accept an in-app request. Content flows only
  between mutual friends.
- **Named hearts** (never a bare count) are the only propagation — no
  reposts. A friend's heart on a stranger's post appears as a greyed-out
  door with a friendship request, not as content.
- **Keeps are leases.** A kept copy dies when the post's lifetime ends or
  its author deletes it.
- **Media:** photos inline, voice notes recorded in-app (waveform travels in
  the post and renders before the audio arrives), videos play inline.

## Platform notes

- **Microphone (macOS):** a denied or missing microphone shows as a disabled
  record button, never a crash.
- **Sandbox:** debug builds run unsandboxed so `JYN_DATA_DIR` and arbitrary
  p2p ports work; release builds are sandboxed with network, microphone, and
  user-selected-file entitlements.

## Trust model (v1)

Friends-only replication is enforced at the application layer, not
cryptographically: profile topics are derivable from profile ids, so a peer
who learns your id could attempt to sync your topic. Private ("only you")
posts are structurally unleakable — they are never serialized into the
replicated log. End-to-end encryption of friend content via
`p2panda-encryption` is the designated next step.

## Design

- `docs/2026-07-03-product-direction.md` — why this exists
- `docs/2026-07-04-screen-design-foundations.md` — the v1
  mental models the screens implement
- `docs/2026-07-05-flutter-port-spec.md` — the Bevy → Flutter port
