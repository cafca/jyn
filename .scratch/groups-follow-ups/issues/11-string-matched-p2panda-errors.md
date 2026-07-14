# 11 — p2panda errors are matched by message substring

**Status:** needs-triage

**Context:** review cleanup deferred from PR #9. `core/src/groups/service.rs`
and `core/src/spaces/mod.rs` retry/skip decisions.

## Problem

Some retry-or-fail decisions branch on `err.to_string().contains(...)`
against p2panda-spaces error messages (e.g. missing-key-bundle /
not-yet-welcomed cases). A wording change in the pinned fork silently turns
a retryable error into a hard failure or vice versa.

## Fix direction

Match on error *types*/variants where the fork exposes them; where it
doesn't, upstream a typed error to `cafca/p2panda` (we already pin a fork
branch, `spaces-scoped-deps`) and pin past it.
