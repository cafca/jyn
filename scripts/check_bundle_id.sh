#!/usr/bin/env bash
#
# Bundle-id guard (ADR 0020): the application id migrated from the old
# reverse-DNS id to `app.jyn.jyn`, and no tracked file may still carry the old
# literal. docs/ and .scratch/ are exempt — the ADR and specs quote the old id
# as history — and this script excludes itself so the pattern it searches for
# isn't a false positive.
#
#   scripts/check_bundle_id.sh
#
# Exits non-zero and lists offenders if the old literal survives anywhere else.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Assembled from parts so this guard file never contains the literal it forbids.
OLD_ID="land.jyn.$(printf jyn)"

# `git ls-files` scopes the search to tracked files only. The exclusions cover
# the docs/specs that legitimately record the migration, and this guard.
hits="$(
  git ls-files \
    | grep -vE '^(docs/|\.scratch/)' \
    | grep -v '^scripts/check_bundle_id.sh$' \
    | xargs grep -Fn "$OLD_ID" 2>/dev/null || true
)"

if [ -n "$hits" ]; then
  echo "!! Old bundle id '${OLD_ID}' still present (ADR 0020 requires app.jyn.jyn):" >&2
  echo "$hits" >&2
  exit 1
fi

echo "OK: no '${OLD_ID}' literal in tracked files (outside docs/.scratch)."
