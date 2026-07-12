# Group membership lifecycle: leave, remove, transfer-only owner exit

**Status:** accepted

- **Leave** — a Member may leave at any time, unconditionally. A leave is an op
  in the Group's log, so the "member from T1 to T2" record is preserved.
- **Remove** — the Owner (sole `Manage` holder) may remove any Member. This is
  the core single-owner governance action.
- **Owner exit — transfer only.** The Owner cannot leave while the Group has
  other members without first **transferring ownership** (reassigning `Manage`
  to another Member), then leaving. Reassigning `Manage` exercises exactly the
  role-reassignment machinery the future multi-admin transition
  ([ADR-0001](0001-single-owner-groups-first.md)) needs, so it is built now.
- **Re-key on removal (Members-only groups)** — when a Member is removed or
  leaves an encrypted Group, future posts must be unreadable to them. Reuse the
  encryption spec's **lazy re-keying** (re-key right before the next Group post,
  bounding re-key to publish frequency, not churn). Content already delivered is
  **not** clawed back — consistent with the existing acceptance that `keep_post`
  lets any friend retain content.

  *Forward note:* the no-clawback contract is tied to encryption **phase 3
  (garbage collection of expired ciphertext + blobs)**. Before merging the
  Groups work, check whether phase 3 has landed on `main`; if so, GC-based
  tightening of removed-member retention comes into scope. See the spec's
  "Pre-merge checks".

## Scope boundary

- **Deleting a Group is out of scope for this phase** — deliberately deferred.
  There is no "delete group" action yet.

## Known gap

- **Sole-owner exit.** If the Owner is the only Member, there is no one to
  transfer to and no delete action, so the Owner cannot cleanly exit; the Group
  becomes dormant. Acceptable for this phase; revisit alongside group deletion.
