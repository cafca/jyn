# 08 — GroupsIngestReport is plumbed but never read

**Status:** needs-triage

**Context:** review cleanup deferred from PR #9.
`core/src/groups/service.rs` (`GroupsIngestReport`, ~110).

## Problem

`ingest`, `drain_pending` and `process_backlog` all build and return a
`GroupsIngestReport` (changed groups, processed count), mirroring the
profile spaces `IngestReport` — but no caller inspects any field; every call
site drops the value or only `?`s the `Result`. It is dead plumbing that
suggests a reactivity mechanism (re-emit state for changed groups) that is
actually driven elsewhere.

## Fix direction

Either use it — have the sync layer re-emit `GroupState` for exactly
`report.changed_groups` instead of the coarser after-change paths — or
delete the struct and return `Result<()>`. Using it dovetails with issue 06
(fewer blanket reductions).
