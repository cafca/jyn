# 10 — Space and group reconcilers duplicate ~28 lines of membership diffing

**Status:** needs-triage

**Context:** review cleanup deferred from PR #9. `core/src/spaces/mod.rs`
(profile space reconcile) vs `core/src/groups/service.rs`
(`reconcile_group_crypto`).

## Problem

Both reconcilers compute the same add/remove diff between a desired member
set and the space's current one and apply it through the manager, ~28 lines
near-identical. A behavioral fix on one side (ordering, error policy,
`remove_stale` nuances) has to be mirrored by hand.

## Fix direction

Extract the diff-and-apply into a shared helper. Subsumed by issue 01 if the
sealed-context primitive lands first — triage together.
