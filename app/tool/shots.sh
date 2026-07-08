#!/usr/bin/env bash
set -euo pipefail
# Screenshot harness runner: boots the built app once per screen with
# JYN_SHOT set, collecting PNGs and failing on any framework error.
# Build first: flutter build macos --debug
cd "$(dirname "$0")/.."

BIN="build/macos/Build/Products/Debug/jyn.app/Contents/MacOS/jyn"
[[ -x "$BIN" ]] || { echo "build first: flutter build macos --debug" >&2; exit 1; }

OUT="${1:-/tmp/jyn-shots}"
mkdir -p "$OUT"

fail=0
for screen in onboarding home composer profile add_friend edit_profile settings diagnostics; do
  data="$(mktemp -d "${TMPDIR:-/tmp}/jyn-shot-data.XXXXXX")"
  if JYN_DATA_DIR="$data" JYN_SHOT="$screen" JYN_SHOT_OUT="$OUT/$screen.png" \
      "$BIN" >"$OUT/$screen.log" 2>&1; then
    echo "OK   $screen"
  else
    echo "FAIL $screen (see $OUT/$screen.log)"
    fail=1
  fi
  rm -rf "$data"
done
exit $fail
