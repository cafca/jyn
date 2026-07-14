#!/usr/bin/env bash
# Restore lockfile-pinned agent skills and expose them where Claude Code indexes them.
#
# `npx skills experimental_install` restores the pinned skills into the canonical
# store at .agents/skills/ (gitignored), but Claude Code only discovers project
# skills under .claude/skills/. This script bridges the two: it restores the store
# when missing, then mirrors each skill into .claude/skills/ as a symlink (the same
# layout `skills add --agent claude-code` produces) so Claude indexes them.
#
# Wired as a SessionStart hook in .claude/settings.json. Idempotent and quiet.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

LOCKFILE="skills-lock.json"
STORE=".agents/skills"
INDEX=".claude/skills"

# Nothing to restore without a lockfile.
[ -f "$LOCKFILE" ] || exit 0

# Restore pinned skills into the canonical store only when it's missing or empty,
# so we don't hit the network on every session.
if [ ! -d "$STORE" ] || [ -z "$(ls -A "$STORE" 2>/dev/null)" ]; then
  npx -y skills@latest experimental_install >/dev/null 2>&1 || exit 0
fi

mkdir -p "$INDEX"

# Mirror each restored skill into the Claude-indexed dir as a symlink.
for skill in "$STORE"/*/; do
  [ -d "$skill" ] || continue
  name="$(basename "$skill")"
  link="$INDEX/$name"
  # Point the symlink at the store, relative to .claude/skills/. Never clobber a
  # real (non-symlink) directory a user may have placed here by hand.
  if [ -L "$link" ] || [ ! -e "$link" ]; then
    ln -sfn "../../$STORE/$name" "$link"
  fi
done

# Prune dangling symlinks for skills dropped from the lockfile.
for link in "$INDEX"/*; do
  [ -L "$link" ] && [ ! -e "$link" ] && rm -f "$link"
done

exit 0
