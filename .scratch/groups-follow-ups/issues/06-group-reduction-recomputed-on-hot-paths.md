# 06 — Group reduction and backlog scans are recomputed on hot paths

**Status:** needs-triage

**Context:** review findings deferred from PR #9 as perf (correct results,
wasted work). All in `core/src/groups/service.rs` and `core/src/sync.rs`.

## Problem

`read_group_state` replays the group's whole operation history; several
paths run it (or full raw scans) more often than needed:

- The maintenance tick runs up to three full reductions per group per pass
  (`process_owner_duties` → view/emit paths in `core/src/sync.rs`, ~747,
  ~768, ~1049) where one could be computed and shared.
- `process_backlog` re-reads `operations_for_group_raw` per group and probes
  `is_processed` per op — an N+1 over sqlite on every startup.
- `JoinGroup` and `reconcile_groups` sweep *all* registered groups when the
  affected group is known.

Cost grows with group count × history length, on the serialized `state.sync`
lock, so it turns into UI latency.

## Fix direction

Pass reduced state down instead of re-deriving it per callee; batch the
`is_processed` probe (one `IN (...)` query per group); scope sweeps to the
group at hand. The reduction cache main removed for profiles
(`REDUCED_PROFILE_STATE_VERSION`) is the bigger hammer if this stays hot.
