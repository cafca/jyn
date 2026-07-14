# Linux ships as AppImage + .deb, built in CI with flutter_distributor

**Status:** accepted

Linux is the first desktop target after macOS. `flutter build linux` needs a
Linux host with the GTK toolchain, but development happens on macOS — so unlike
the macOS release (cut locally by `scripts/release.sh` because notarization
needs a Mac), the Linux build has to run **in CI**. Two properties shape the
packaging: the p2panda node binds arbitrary ports and does LAN/mDNS peer
discovery, and `media_kit` (ADR 0018) pulls **libmpv** in as a runtime
dependency.

We will produce **two artifacts via `flutter_distributor`** and attach them to
the existing GitHub Release: an **AppImage** (bundles GTK + libmpv, single
self-contained file, no sandbox) and a **`.deb`** (declares its runtime deps).
Builds run on **`ubuntu-22.04`** (glibc 2.35), which sets the compatibility
floor at current LTS / Debian stable. A tag push triggers the release workflow,
which reads the same `VERSION` file the macOS flow uses; a PR-level Linux build
job guards against regressions. This is a **distributable** target, not
auto-updating parity — there is no Sparkle equivalent, so Linux updates are a
manual re-download.

## Considered Options

- **(A) AppImage + .deb via flutter_distributor** — chosen. AppImage's
  no-sandbox model suits a p2p app that needs arbitrary ports and mDNS; the
  `.deb` serves apt users; the tool bundles libmpv/GTK for us.
- **(B) Flatpak** — rejected for now. The sandbox fights arbitrary-port
  networking, mDNS discovery, and the portal file picker — real work for no
  benefit at friends-circle scale. Revisit if a managed update channel is
  wanted.
- **(C) Hand-rolled packaging scripts** — rejected. `flutter_distributor`
  covers exactly AppImage + deb including the fiddly libmpv/AppDir bundling;
  hand-rolling is reserved for the Apple-only notarization/Sparkle steps that
  no tool does.

## Consequences

- The release process is now **split**: macOS is cut locally (`release.sh`,
  needs a Mac for notarization), Linux is cut in CI on a version tag. Both are
  keyed to `VERSION`, so a release stays coherent.
- The glibc-2.35 floor excludes pre-2022 distributions.
- The AppImage has no auto-update; that is an accepted property of target (B).
