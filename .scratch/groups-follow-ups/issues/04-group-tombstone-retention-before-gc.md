# 04 — A group tombstone can be GC'd before offline members see it

**Status:** needs-triage

**Context:** the group instance of
`.scratch/co-deletion-logs-gc/issues/01-tombstone-retention-before-gc.md`,
inherited by the ADR-0018 bucketing: group buckets now drain via the same
`drop_drained_buckets`, wired into `drain_expired` (`core/src/bridge.rs`).

## Problem

Deleting a permanent group post writes its `PostDeleted` tombstone into the
post's permanent-month bucket. Both the post and its tombstone classify dead,
so the author's next drain can prune the whole bucket — tombstone included —
with no window forcing a replication delay. A member who was offline across
that gap never receives the tombstone; their locally-synced copy of the
bucket keeps the post in their group reduction indefinitely (groups have no
keep leases to eventually expire it, unlike the profile case).

## Fix direction

Whatever resolution the profile-side ticket lands (tombstone grace period,
tombstones on a non-drainable log, or head-pointer-based convergence) should
be applied to group buckets in the same change — the mechanism is shared.
Blocked by: that ticket's triage.
