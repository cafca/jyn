#!/usr/bin/env bash
#
# Linux smoke test (linux-desktop-builds spec): prove the freshly built app
# actually *runs*, not merely that it compiled. Launch the binary under a
# virtual display with a throwaway data directory, let it live a few seconds,
# then signal it to quit — asserting it never crashed on its own and printed no
# error or panic lines. A clean run means the Rust core linked, libmpv loaded,
# and GTK initialised.
#
#   app/tool/linux_smoke_test.sh [path/to/jyn]
#
# Defaults to the release bundle produced by `flutter build linux --release`.
# Requires Xvfb on PATH (the CI runner installs it).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_ROOT="$(cd "$HERE/.." && pwd)"
BIN="${1:-$APP_ROOT/build/linux/x64/release/bundle/jyn}"

RUN_SECONDS="${SMOKE_RUN_SECONDS:-8}"
QUIT_TIMEOUT="${SMOKE_QUIT_TIMEOUT:-15}"

if [ ! -x "$BIN" ]; then
  echo "!! smoke: binary not found or not executable: $BIN" >&2
  exit 1
fi
if ! command -v Xvfb >/dev/null 2>&1; then
  echo "!! smoke: Xvfb not found on PATH (install xvfb)" >&2
  exit 1
fi

WORK="$(mktemp -d)"
LOG="$WORK/output.log"
DATA_DIR="$WORK/data"
mkdir -p "$DATA_DIR"

# Own display number so parallel jobs don't collide; :99 is the conventional
# CI headless display.
DISPLAY_NUM="${SMOKE_DISPLAY:-99}"

XVFB_PID=""
APP_PID=""
cleanup() {
  [ -n "$APP_PID" ] && kill -KILL "$APP_PID" 2>/dev/null || true
  [ -n "$XVFB_PID" ] && kill -KILL "$XVFB_PID" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

echo "==> smoke: launching $BIN"
Xvfb ":$DISPLAY_NUM" -screen 0 1280x720x24 >/dev/null 2>&1 &
XVFB_PID=$!
sleep 1

# A throwaway data dir keeps the run hermetic and off the real profile
# (JYN_DATA_DIR is honoured by the node; see lib/src/rust/api/lifecycle.dart).
DISPLAY=":$DISPLAY_NUM" JYN_DATA_DIR="$DATA_DIR" "$BIN" >"$LOG" 2>&1 &
APP_PID=$!

# Let it come up and settle.
sleep "$RUN_SECONDS"

# If it already exited, it crashed or bailed on its own — a failure.
if ! kill -0 "$APP_PID" 2>/dev/null; then
  wait "$APP_PID" 2>/dev/null; code=$?
  echo "!! smoke: app exited on its own within ${RUN_SECONDS}s (code $code)" >&2
  echo "----- output -----" >&2; cat "$LOG" >&2
  exit 1
fi

# Still alive: signal a graceful quit and wait for it to go.
echo "==> smoke: app alive after ${RUN_SECONDS}s, sending SIGTERM"
kill -TERM "$APP_PID" 2>/dev/null || true
for _ in $(seq 1 "$QUIT_TIMEOUT"); do
  kill -0 "$APP_PID" 2>/dev/null || break
  sleep 1
done
if kill -0 "$APP_PID" 2>/dev/null; then
  echo "!! smoke: app ignored SIGTERM after ${QUIT_TIMEOUT}s" >&2
  echo "----- output -----" >&2; cat "$LOG" >&2
  exit 1
fi
APP_PID=""

# Scan the captured output for real failure signals. The core logs at INFO to
# stderr by default (tracing), so we look for crash/error markers specifically
# rather than the substring "error" — tracing's ERROR level, Rust panics, Dart
# exceptions, and dynamic-loader failures (a libmpv/GTK that didn't load).
FAIL_RE='\bpanic|panicked|Segmentation fault|core dumped|Unhandled exception|EXCEPTION CAUGHT|error while loading shared libraries|cannot open shared object|failed to load dynamic library'
LEVEL_RE=' ERROR '
if grep -Eiq "$FAIL_RE" "$LOG" || grep -q "$LEVEL_RE" "$LOG"; then
  echo "!! smoke: error/panic lines in output:" >&2
  grep -Ein "$FAIL_RE" "$LOG" >&2 || true
  grep -n "$LEVEL_RE" "$LOG" >&2 || true
  exit 1
fi

echo "==> smoke: OK — started, ran ${RUN_SECONDS}s, quit cleanly, no error/panic lines"
