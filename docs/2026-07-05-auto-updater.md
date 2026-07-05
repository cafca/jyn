# Auto-updater (macOS, Sparkle)

jyn updates itself on macOS via [Sparkle](https://sparkle-project.org),
driven from Dart by the [`auto_updater`](https://pub.dev/packages/auto_updater)
plugin. This is a desktop concern only: iOS and Android update through their
app stores; Linux and Windows get backends as those ports land (the plugin
already wraps WinSparkle, so Windows is largely free later).

## Decisions

- **Framework:** Sparkle via `auto_updater` (macOS today, Windows later).
- **Distribution:** notarized `.zip` published as a GitHub Release asset.
- **Feed:** a latest-only appcast served from a fixed GitHub Pages URL,
  `https://cafca.github.io/jyn/appcast.xml`, regenerated every release.
- **Release pipeline:** a local script (`scripts/release.sh`) run on the
  release Mac — not CI. Uses the existing Developer ID + notary setup; the
  Sparkle EdDSA private key stays in the login Keychain.
- **UX:** automatic checks on launch + every 24h, plus a native
  "Check for Updates…" item in the application menu. Prompt-based, never
  silent — the user always sees version, notes, and Install/Later.
- **Sandbox:** release builds stay sandboxed. Sparkle runs inside the sandbox
  via its installer XPC service (`SUEnableInstallerLauncherService` +
  mach-lookup exceptions in `Release.entitlements`); no downloader XPC is
  needed because `com.apple.security.network.client` is already granted.
- **Versioning:** the repo-root `VERSION` file (semver) is the single source
  of truth. `scripts/release_tools.py` projects it into pubspec's
  `version: <semver>+<build>`, where `build = git rev-list --count HEAD` is the
  monotonic integer Sparkle compares (CFBundleVersion).

## Explicitly out of scope (v1)

Critical/minimum-version update gating, silent background installs, delta
updates, and core-side p2p protocol-version enforcement (peers on an
incompatible schema). The last is noted as separate future work: Sparkle can
only *encourage* upgrades; refusing incompatible peers belongs in the core's
handshake.

## Where things live

| Concern | File |
| --- | --- |
| Updater init, feed URL, daily interval, menu bridge | `app/lib/main.dart` |
| Native "Check for Updates…" menu item | `app/macos/Runner/MainFlutterWindow.swift` |
| Sparkle keys (feed, public key, XPC service) | `app/macos/Runner/Info.plist` |
| Sandbox mach-lookup exceptions | `app/macos/Runner/Release.entitlements` |
| Version source of truth | `VERSION` |
| Version projection + appcast rendering (tested) | `scripts/release_tools.py`, `scripts/test_release_tools.py` |
| Release pipeline | `scripts/release.sh` |
| Served appcast | `docs/appcast.xml` |

## One-time setup (release machine)

1. **Generate the Sparkle ed25519 keypair** (stores the private key in the
   login Keychain, prints the public key):
   ```
   cd app && dart run auto_updater:generate_keys
   ```
   Paste the printed public key into `app/macos/Runner/Info.plist` replacing
   `REPLACE_WITH_SPARKLE_ED_PUBLIC_KEY`. Back up the private key
   (`dart run auto_updater:generate_keys --export`) somewhere safe — losing it
   means no client can verify future updates.
2. **Store notarization credentials** as a keychain profile (App Store Connect
   API key or app-specific password):
   ```
   xcrun notarytool store-credentials jyn-notary
   ```
3. **Confirm the Developer ID signing identity** is installed:
   `security find-identity -v -p codesigning` should list
   `Developer ID Application: …`.
4. **Enable GitHub Pages** for `cafca/jyn`: Settings → Pages → Deploy from
   branch → `main` / `/docs`. After the first push the appcast is live at
   `https://cafca.github.io/jyn/appcast.xml`.

## Cutting a release

1. Bump `VERSION` (e.g. `1.0.1`) and commit.
2. Optionally write release notes as an HTML fragment
   (e.g. `dist/notes.html`). For a data-wiping schema change, say so plainly —
   there is no automatic critical-update flag.
3. Run:
   ```
   scripts/release.sh [dist/notes.html]
   ```
   It builds, signs, notarizes, staples, EdDSA-signs, regenerates
   `docs/appcast.xml`, publishes the GitHub Release, and pushes. Users see the
   new version on their next scheduled check (or via "Check for Updates…").

## Verifying the update flow

Sparkle's XPC/sandbox handshake only exercises on a real notarized, sandboxed
build, so verify end-to-end at least once:

1. Release `1.0.0`, install that `.app`, and run it.
2. Bump to `1.0.1`, release again.
3. In the `1.0.0` app, use "Check for Updates…" and confirm it detects,
   downloads, verifies the signature, installs, and relaunches into `1.0.1`.

For faster iteration you can point `SUFeedURL` at a local file server serving a
test appcast instead of GitHub Pages.

## Gotcha: "Apple could not verify …" after a manual download

If a freshly downloaded build is rejected by Gatekeeper with *"Apple could not
verify 'jyn.app' is free of malware"* and `spctl -a -vvv -t exec` reports
**"unsealed contents present in the root directory of an embedded framework"**,
the app itself is fine — extracting the zip (with `unzip` or some browsers)
externalized extended attributes into `._*` AppleDouble sidecars inside the
framework roots, breaking the seal. Clean them and it verifies again:

```
dot_clean -m /Applications/jyn.app   # removes the ._* sidecars
spctl -a -vvv -t exec /Applications/jyn.app   # -> accepted, Notarized Developer ID
```

`release.sh` guards against this by stripping xattrs before signing
(`xattr -cr`) and archiving with `ditto --noextattr --norsrc`, so any extraction
tool stays clean. Sparkle's own update unarchiver already handles this, so the
auto-update path is unaffected — it only bit the first manual install.
