#!/usr/bin/env bash
#
# Local release pipeline for the jyn macOS app: build → codesign (Developer ID)
# → notarize → staple → zip → Sparkle EdDSA-sign → regenerate the appcast →
# publish a GitHub Release → commit the appcast to docs/ (GitHub Pages).
#
# Run from the repo root on the release Mac, once your one-time setup is done
# (see docs/2026-07-05-auto-updater.md): the ed25519 private key is in the
# login Keychain, `xcrun notarytool store-credentials "$NOTARY_PROFILE"` has
# been run, and SUPublicEDKey is set in app/macos/Runner/Info.plist.
#
#   scripts/release.sh [path/to/release-notes.html]
#
# The version comes from the VERSION file; bump and commit it first.
set -euo pipefail

# --- config (override via env) ----------------------------------------------
DEVELOPER_ID="${DEVELOPER_ID:-Developer ID Application}"  # signing identity
NOTARY_PROFILE="${NOTARY_PROFILE:-jyn-notary}"            # notarytool keychain profile
REPO="${REPO:-cafca/jyn}"
MIN_MACOS="${MIN_MACOS:-11.0}"
# ----------------------------------------------------------------------------

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

NOTES_FILE="${1:-}"
SEMVER="$(tr -d '[:space:]' < VERSION)"
BUILD="$(git rev-list --count HEAD)"
TAG="v${SEMVER}"
APP="app/build/macos/Build/Products/Release/jyn.app"
DIST="$ROOT/dist"
ZIP="${DIST}/jyn-${SEMVER}.zip"
ASSET_URL="https://github.com/${REPO}/releases/download/${TAG}/jyn-${SEMVER}.zip"

echo "==> Releasing jyn ${SEMVER} (build ${BUILD}, tag ${TAG})"

if grep -q REPLACE_WITH_SPARKLE_ED_PUBLIC_KEY app/macos/Runner/Info.plist; then
  echo "!! SUPublicEDKey is still a placeholder. Run:" >&2
  echo "   (cd app && dart run auto_updater:generate_keys)" >&2
  echo "   then paste the public key into app/macos/Runner/Info.plist." >&2
  exit 1
fi

# 1. Project VERSION into the Flutter build numbers.
python3 scripts/release_tools.py sync-pubspec "$SEMVER" "$BUILD" app/pubspec.yaml

# 2. Build the release app (cargokit compiles the Rust core).
(cd app && flutter build macos --release)

# 3. Strip extended attributes / AppleDouble sidecars first: otherwise archiving
#    can externalize them into ._* files that reappear on extraction inside the
#    framework roots, which breaks the code seal ("unsealed contents present in
#    the root directory of an embedded framework"). Then re-sign with Developer
#    ID + hardened runtime.
dot_clean -m "$APP" 2>/dev/null || true
xattr -cr "$APP"
codesign --force --deep --options runtime --timestamp \
  --sign "$DEVELOPER_ID" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

# 4. Zip, notarize, staple.
mkdir -p "$DIST"
rm -f "$ZIP"
/usr/bin/ditto -c -k --keepParent --noextattr --norsrc "$APP" "$ZIP"
echo "==> Notarizing (this can take a few minutes)…"
xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$APP"
# Re-zip so the distributed archive carries the stapled ticket.
rm -f "$ZIP"
/usr/bin/ditto -c -k --keepParent --noextattr --norsrc "$APP" "$ZIP"

# 5. Sparkle EdDSA signature over the final archive.
SIGN_OUT="$(cd app && dart run auto_updater:sign_update "$ZIP")"
echo "    sign_update: $SIGN_OUT"
ED_SIG="$(sed -nE 's/.*sparkle:edSignature="([^"]+)".*/\1/p' <<<"$SIGN_OUT")"
LENGTH="$(sed -nE 's/.*length="([0-9]+)".*/\1/p' <<<"$SIGN_OUT")"
[ -n "$ED_SIG" ] && [ -n "$LENGTH" ] || {
  echo "!! could not parse sign_update output" >&2; exit 1; }

# 6. Regenerate the appcast (latest-only).
NOTES_HTML=""
if [ -n "$NOTES_FILE" ]; then NOTES_HTML="$(cat "$NOTES_FILE")"; fi
PUB_DATE="$(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S +0000')"
python3 - "$SEMVER" "$BUILD" "$ASSET_URL" "$ED_SIG" "$LENGTH" "$PUB_DATE" "$MIN_MACOS" "$NOTES_HTML" <<'PY' > docs/appcast.xml
import json, sys
sys.path.insert(0, "scripts")
import release_tools as rt
semver, build, url, sig, length, pub_date, min_macos, notes = sys.argv[1:9]
item = {"short_version": semver, "build": int(build), "url": url,
        "ed_signature": sig, "length": int(length), "pub_date": pub_date,
        "minimum_system_version": min_macos}
if notes:
    item["notes_html"] = notes
sys.stdout.write(rt.render_appcast([item]))
PY
echo "==> Wrote docs/appcast.xml"

# 7. Publish the GitHub Release with the notarized zip as an asset.
git tag -f "$TAG"
git push -f origin "$TAG"
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  gh release upload "$TAG" "$ZIP" --repo "$REPO" --clobber
else
  gh release create "$TAG" "$ZIP" --repo "$REPO" \
    --title "jyn ${SEMVER}" \
    --notes "${NOTES_FILE:+$(cat "$NOTES_FILE")}"
fi

# 8. Commit + push the appcast so GitHub Pages serves the new version.
git add docs/appcast.xml app/pubspec.yaml
git commit -m "Release ${SEMVER}"
git push origin HEAD

echo "==> Done. Users will see ${SEMVER} on their next scheduled check."
