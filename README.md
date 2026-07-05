# jyn

A p2p board for a small circle of friends, built on p2panda. Posts flow down
one river; the only thing separating them is their author-chosen lifetime —
ephemeral posts drain away, permanent ones settle. The poster is sovereign,
not the platform: edits are marked, deletes reach every copy (kept ones
included), and nobody outside the friendship can see or erase anything.

Experimental. Expect breaking changes with any version update; on-disk
stores are wiped on schema mismatches (only `node.key` and `settings.json`
survive).

## Running

```
cargo run
```

Two instances side by side for local testing:

```
JYN_DATA_DIR=/tmp/jyn-a cargo run
JYN_DATA_DIR=/tmp/jyn-b cargo run
```

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
  the post and renders before the audio arrives), videos play inline when
  `ffmpeg` is installed and degrade to open-externally file chips otherwise.

## Platform notes

- **Microphone (macOS):** at dev-run time the permission prompt attributes
  to your terminal. A denied or missing microphone shows as a disabled
  record button, never a crash.
- **Inline video** shells out to `ffmpeg`/`ffprobe` on your PATH
  (`brew install ffmpeg`). No build-time ffmpeg dependency.
- **Fonts:** drop `SpaceMono-Regular.ttf` into `assets/fonts/` (OFL) to get
  the design's mono HUD; egui's built-in monospace is the fallback.

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
