# Spec — Linux desktop builds (AppImage + .deb)

**Status:** ready-for-agent

Governed by ADR 0019 (Linux distribution) and ADR 0020 (bundle-id migration).
**Depends on** the media_kit-migration spec landing and being macOS-verified
first.

## Problem Statement

As someone who runs Linux, I can't install or run jyn at all — it only ships for
macOS, so I'm shut out of the friendship. As the maintainer, I develop on macOS
and have no Linux machine to build on, and the application's identifier
(`land.jyn.jyn`) no longer matches the project's domain (jyn.app).

## Solution

jyn ships downloadable Linux builds — a self-contained AppImage and a `.deb` —
attached to each GitHub Release and built automatically in CI. A friend on a
current Linux distribution downloads one file and runs it; posting, reading,
voice notes, video, and peer sync all work. The application identifier matches
the jyn.app domain.

## User Stories

1. As a Linux user, I want a single self-contained file (AppImage) I can
   download and run without installing dependencies, so that trying jyn is
   frictionless.
2. As a Linux user on Debian/Ubuntu, I want a `.deb` I can install through my
   package manager, so that jyn integrates like a native app.
3. As a Linux user, I want the app to appear in my launcher with jyn's name and
   icon, so that it feels like a real desktop application.
4. As a Linux user, I want to create a post, read the river, record and play a
   voice note, and play a video, so that I have full parity with the macOS app's
   media.
5. As a Linux user, I want to become friends and sync posts with a peer on my
   local network, so that the p2p core works on my platform.
6. As a Linux user on a current LTS or Debian stable release, I want the build to
   actually launch on my glibc version, so that the download isn't dead on
   arrival.
7. As the maintainer, I want the Linux build produced in CI without a Linux
   machine of my own, so that I can cut releases from macOS.
8. As the maintainer, I want a Linux build to run on every pull request, so that
   a change that breaks the Linux target is caught before merge.
9. As the maintainer, I want the Linux release keyed to the same VERSION file as
   the macOS release, so that a version is coherent across platforms.
10. As the maintainer, I want the app's identifier to be `app.jyn.jyn`, so that
    it matches the jyn.app domain.
11. As the maintainer, I want an automated check that the freshly built Linux app
    starts and exits cleanly, so that "it compiled" is upgraded to "it runs."

## Implementation Decisions

- Produce two artifacts with flutter_distributor: an **AppImage** (bundles GTK +
  libmpv, single file, no sandbox — chosen so the p2p node's arbitrary ports and
  LAN/mDNS discovery aren't fought by a sandbox) and a **`.deb`**; attach both to
  the GitHub Release.
- Builds run in CI on `ubuntu-22.04` (glibc 2.35), setting the compatibility
  floor at current LTS / Debian stable (per ADR 0019).
- A pull-request Linux build job gates regressions. A version-tag-triggered
  release workflow reads the shared VERSION file, builds the artifacts, and
  uploads them to the Release.
- The release process is split: macOS is cut locally (needs a Mac for
  notarization), Linux is cut in CI; both keyed to VERSION.
- No auto-update on Linux; updates are a manual re-download (accepted property of
  a distributable, non-parity target).
- Desktop entry and icon reuse the existing macOS application icon; application
  id is `app.jyn.jyn`.
- Bundle-id change (ADR 0020): change the identifier from `land.jyn.jyn` to
  `app.jyn.jyn` in the macOS build configuration, the updater method-channel
  names, and the Linux application id. Sparkle mach-service names auto-derive
  from the bundle id and follow automatically. This version discards all prior
  on-disk data, so there is no identity or data migration to perform.
- Whether libmpv is bundled into the `.deb` or declared as a dependency is
  decided once media_kit's shipped Linux libraries are inspected.

## Testing Decisions

- A good test asserts external behavior — the process starts and exits cleanly,
  the artifacts exist and are installable — not build internals.
- **Automated CI smoke test:** on the Linux runner, under a virtual display, the
  freshly built app is launched with a temporary data directory, left running a
  few seconds, then signalled to quit. The test asserts it exits cleanly with no
  crash and **no error or panic lines in stdout/stderr**. This proves the core
  links, libmpv loads, and GTK initializes — not merely that it compiled.
- **Bundle-id guard:** assert no `land.jyn.jyn` literal remains anywhere in the
  tree, and the app builds and launches under `app.jyn.jyn`.
- **Manual acceptance** on a clean 22.04-era distribution: install the AppImage,
  and separately the `.deb`; drive post/read → record and play a voice note →
  play a video → LAN sync with a second peer. This is where `record`'s Linux
  capture backend is verified.

## Out of Scope

- Windows — a documented fast-follow that reuses this work (media_kit and
  flutter_distributor both cover it, and it gets WinSparkle for free).
- iOS/Android.
- Auto-update or a managed update channel for Linux.
- Flatpak and snap packaging (rejected for now in ADR 0019).
- Touch/mobile UI adaptation.
- The media_kit migration itself — a separate, prerequisite spec.

## Further Notes

- Old GitHub releases carry the previous identifier and are left in place as
  harmless history; the Sparkle appcast is latest-only and regenerated on every
  release, so it is unaffected.
- The glibc-2.35 floor excludes pre-2022 distributions — an accepted trade for a
  stock runner and broad current-distro coverage.
