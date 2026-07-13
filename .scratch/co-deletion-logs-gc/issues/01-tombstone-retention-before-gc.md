# 01 — A deleted post's tombstone can be GC'd before offline peers see it

**Status:** needs-triage

**Context:** Co-deletion-logs GC (ADR-0016, `docs/adr/0016-logs-are-expiry-keyed-co-deletion-units.md`).
Surfaced while implementing the bucket-drop GC (`JynOperationDomain::drop_drained_buckets`,
`core/src/domain.rs`; wired into `drain_expired` in `core/src/bridge.rs`). Not one of
the reviewed bugs — a separate gap noticed in passing.

## Problem

Deleting a post writes a `PostDeleted` tombstone into the post's bucket log. That
tombstone is the mechanism that reaches into readers' **kept** copies: `drain_expired`'s
keep-lease enforcement prunes a keep when `author_state.is_tombstoned(post_id)` is true.

But bucket-drop GC treats a `PostDeleted` op as always-dead, so a bucket containing only
a (now-tombstoned) post plus its tombstone becomes **immediately drainable** — GC can
`prune_entries` the whole log, tombstone included, on the very next drain pass. For an
**expiry** bucket this is fine: the window has to pass first, by which time the tombstone
has long since replicated. For a **permanent** post deleted by the author, there is no
window — the bucket drains on the next drain after the delete.

If an offline follower/keeper has not yet synced the tombstone when the author's GC drops
it, that peer never learns of the delete and its kept copy survives until the keep's own
lease lapses. This weakens the "delete reaches kept copies" promise for offline recipients.

### Failure scenario

1. Alice publishes a **permanent** Friends post P. Bob is a friend and **keeps** P (his own
   `keep/…` snapshot).
2. Bob goes offline.
3. Alice deletes P → `PostDeleted` into P's permanent bucket.
4. Alice's next drain (or restart) runs GC; the bucket holds only P + its tombstone, both
   dead → the whole log is pruned and un-associated.
5. Bob comes back online. Alice's log for that bucket is gone, so catch-up sync never
   delivers the tombstone. Bob's kept copy of P outlives Alice's delete, indefinitely,
   until Bob's keep lease lapses.

(Online followers are unaffected — the tombstone reaches them via live gossip at delete
time. The gap is specifically offline peers relying on catch-up sync.)

## Candidate approaches (for triage — pick one)

- **Tombstone retention grace period.** Keep a bucket that is dead *only because of a
  tombstone* for a bounded window (e.g. one coarse chip / N days) before it is drop-eligible,
  giving offline peers time to catch up. Simple; bounded over-retention of header-only
  metadata.
- **Separate tombstone stream.** Route `PostDeleted` into a longer-lived (or reserved)
  log that isn't window-dropped, so tombstones persist independently of their post's bucket.
  Cleaner propagation guarantee; more moving parts, and grows unboundedly without its own GC.
- **Accept as-is (`wontfix`).** Decide that a keep is "its own promise to retain" and that
  a delete is not obligated to revoke an offline peer's keep — the keep already survives
  expiry by design, so this only extends that stance to delete. Document and close.

## Acceptance criteria (once an approach is chosen)

- [ ] A permanent post deleted by its author still has its delete reach an offline
      follower/keeper that syncs within the agreed retention bound; the kept copy is then
      pruned by keep-lease enforcement.
- [ ] Over-retention introduced by the fix is bounded and header-only (no ciphertext, no
      decryptable content lingers beyond existing teardown).
- [ ] Expiry-bucket drop timing is unchanged (this only concerns tombstone-triggered drops).
- [ ] Deterministic test at the domain/bridge seam — no wall-clock waits — proving the
      tombstone survives long enough and the bucket still eventually drops.
