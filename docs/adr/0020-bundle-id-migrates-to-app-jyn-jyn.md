# Bundle id migrates from land.jyn.jyn to app.jyn.jyn

**Status:** accepted

The project domain moved from **jyn.land** to **jyn.app**, so the reverse-DNS
application id should follow: `land.jyn.jyn` → `app.jyn.jyn` (only the leading
TLD segment changes). This version discards all prior on-disk data — identity
included — so there is **nothing to preserve** across the change and no
migration requirement of any kind.

We will change the id to **`app.jyn.jyn`** everywhere it is written literally:
the macOS `PRODUCT_BUNDLE_IDENTIFIER` (xcconfig + pbxproj), the two hardcoded
`land.jyn.jyn/updater` `MethodChannel` strings (`app/lib/main.dart`,
`app/macos/Runner/MainFlutterWindow.swift`), and the Linux `APPLICATION_ID`
(`app/linux/CMakeLists.txt`). Sparkle's mach-service entitlements auto-derive
from `$(PRODUCT_BUNDLE_IDENTIFIER)`, so they follow automatically.

## Consequences

- Existing GitHub releases carry the old id; they are left in place as harmless
  history. The Sparkle appcast is latest-only and regenerated on every release,
  so it is unaffected by the change.
